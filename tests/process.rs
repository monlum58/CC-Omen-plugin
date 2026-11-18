//! Runs the real plugin binary against a fake sysfs and talks to it over its Unix socket,
//! the way CoolerControl does.

use hyper_util::rt::TokioIo;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

pub mod models {
    pub mod v1 {
        tonic::include_proto!("coolercontrol.models.v1");
    }
}
pub mod device_service {
    pub mod v1 {
        tonic::include_proto!("coolercontrol.device_service.v1");
    }
}
use device_service::v1::device_service_client::DeviceServiceClient;
use device_service::v1::*;

const BIN: &str = env!("CARGO_BIN_EXE_cc_omen");

struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new(board: &str) -> Self {
        let f = Self { dir: tempfile::tempdir().unwrap() };
        f.hwmon("hwmon2", "k10temp", &[("temp1_input", "45000")]);
        f.hwmon("hwmon8", "hp", &[("pwm1_enable", "1"), ("pwm1", "85"), ("pwm2", "85"), ("fan1_input", "2000"), ("fan2_input", "2100")]);
        std::fs::create_dir_all(f.root().join("class/dmi/id")).unwrap();
        std::fs::write(f.root().join("class/dmi/id/board_name"), format!("{board}\n")).unwrap();
        f
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn hwmon(&self, hw: &str, name: &str, files: &[(&str, &str)]) {
        let d = self.root().join("class/hwmon").join(hw);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
        for (k, v) in files {
            std::fs::write(d.join(k), v).unwrap();
        }
    }
    fn get(&self, f: &str) -> String {
        std::fs::read_to_string(self.root().join("class/hwmon/hwmon8").join(f)).unwrap().trim().to_string()
    }
    fn set(&self, f: &str, v: &str) {
        std::fs::write(self.root().join("class/hwmon/hwmon8").join(f), v).unwrap();
    }
    fn socket(&self) -> PathBuf {
        self.root().join("plugin.sock")
    }
    fn spawn(&self, extra: &[&str]) -> Child {
        Command::new(BIN)
            .arg("--sysfs-root")
            .arg(self.root())
            .arg("--socket")
            .arg(self.socket())
            .arg("--config")
            .arg(self.root().join("none.json"))
            .args(extra)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn client(sock: PathBuf) -> DeviceServiceClient<Channel> {
    let ch = Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let sock = sock.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(sock).await?)) }
        }))
        .await
        .unwrap();
    DeviceServiceClient::new(ch)
}

/// Waits for a plugin that is expected to refuse to start. One that keeps running is killed
/// and fails the test instead of hanging it.
fn stderr_of(mut child: Child) -> String {
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("the plugin kept running instead of refusing to start");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn process_full_lifecycle() {
    let f = Fake::new("8BCA");
    let mut child = f.spawn(&[]);
    wait_for("socket", || f.socket().exists());

    // ISC-37: firmware auto is written before serving.
    assert_eq!(f.get("pwm1_enable"), "2");
    // ISC-39: the socket is 0600 and owned by the plugin user.
    let meta = std::fs::metadata(f.socket()).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(meta.uid(), unsafe { libc::geteuid() });

    let mut c = client(f.socket()).await;
    // ISC-7 shape: one device, two 0–100 % speed channels with fixed duty.
    let devs = c.list_devices(ListDevicesRequest {}).await.unwrap().into_inner().devices;
    assert_eq!(devs.len(), 1);
    let info = devs[0].info.as_ref().unwrap();
    let mut ids: Vec<_> = info.channels.keys().cloned().collect();
    ids.sort();
    assert_eq!(ids, ["fan1", "fan2"]);
    for ch in info.channels.values() {
        match ch.options.as_ref().unwrap() {
            models::v1::channel_info::Options::SpeedOptions(o) => assert!(o.fixed_enabled && o.min_duty == 0 && o.max_duty == 100),
            other => panic!("not a speed channel: {other:?}"),
        }
    }
    let dev = devs[0].id.clone();

    // ISC-11: a duty above 0 % switches to manual and writes both channels.
    c.fixed_duty(FixedDutyRequest { device_id: dev.clone(), channel_id: "fan1".into(), duty: 40 }).await.unwrap();
    assert_eq!((f.get("pwm1_enable"), f.get("pwm1"), f.get("pwm2")), ("1".into(), "102".into(), "0".into()));

    // ISC-8: Status reports the RPM and the driver's pwm as a percent.
    f.set("pwm1", "85");
    wait_for("a control cycle", || {
        std::thread::sleep(Duration::from_millis(1100));
        true
    });
    let st = c.status(StatusRequest { device_id: dev.clone() }).await.unwrap().into_inner().status;
    let fan1 = st.iter().find(|s| s.id == "fan1").unwrap();
    match fan1.metric.as_ref().unwrap() {
        models::v1::status::Metric::Speed(s) => {
            assert_eq!(s.rpm, Some(2000));
            assert_eq!(s.duty, Some(33.0));
        }
        other => panic!("{other:?}"),
    }

    // ISC-46: request fields never reach a path; unknown ids are refused.
    let bad = c.fixed_duty(FixedDutyRequest { device_id: dev.clone(), channel_id: "../../pwm1_enable".into(), duty: 50 }).await;
    assert!(bad.is_err());

    // ISC-16: SIGTERM hands the fans back to the firmware and removes the socket.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    assert_eq!(f.get("pwm1_enable"), "2");
    assert!(!f.socket().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn process_shutdown_rpc_hands_fans_back() {
    let f = Fake::new("8BCA");
    let mut child = f.spawn(&[]);
    wait_for("socket", || f.socket().exists());
    let mut c = client(f.socket()).await;
    c.fixed_duty(FixedDutyRequest { device_id: "omen_fans".into(), channel_id: "fan2".into(), duty: 70 }).await.unwrap();
    assert_eq!(f.get("pwm1_enable"), "1");
    c.shutdown(ShutdownRequest {}).await.unwrap();
    assert_eq!(f.get("pwm1_enable"), "2");
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    assert!(child.wait().unwrap().success());
}

#[test]
fn process_refuses_a_foreign_board_unless_allowed() {
    let f = Fake::new("8BC9");
    let child = f.spawn(&[]);
    let err = stderr_of(child);
    assert!(err.contains("8BC9") && err.contains("--allow-any-board"), "{err}");
    assert_eq!(f.get("pwm1_enable"), "1", "nothing written on refusal");

    let mut child = f.spawn(&["--allow-any-board"]);
    wait_for("socket", || f.socket().exists());
    assert_eq!(f.get("pwm1_enable"), "2");
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert!(child.wait().unwrap().success());
}

#[test]
fn process_refuses_without_the_fan_interface() {
    let f = Fake::new("8BCA");
    std::fs::remove_file(f.root().join("class/hwmon/hwmon8/pwm2")).unwrap();
    let err = stderr_of(f.spawn(&[]));
    assert!(err.contains("refusing to start") && err.contains("pwm2"), "{err}");
}

#[test]
fn process_refuses_without_a_cpu_sensor() {
    let f = Fake::new("8BCA");
    std::fs::remove_dir_all(f.root().join("class/hwmon/hwmon2")).unwrap();
    let err = stderr_of(f.spawn(&[]));
    assert!(err.contains("CPU temperature"), "{err}");
}

#[test]
fn process_never_deletes_a_non_socket_at_the_socket_path() {
    let f = Fake::new("8BCA");
    std::fs::write(f.socket(), "precious").unwrap();
    let err = stderr_of(f.spawn(&[]));
    assert!(err.contains("not a socket"), "{err}");
    assert_eq!(std::fs::read_to_string(f.socket()).unwrap(), "precious");

    let g = Fake::new("8BCA");
    let target = g.root().join("target.txt");
    std::fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, g.socket()).unwrap();
    let err = stderr_of(g.spawn(&[]));
    assert!(err.contains("not a socket"), "{err}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}
