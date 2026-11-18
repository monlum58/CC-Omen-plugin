//! The CoolerControl device-service API. Every call feeds the watchdog; channel and device ids
//! are only compared against fixed strings and never reach a filesystem path.

use crate::device_service::v1::device_service_server::DeviceService;
use crate::device_service::v1::{
    CustomFunctionOneRequest, CustomFunctionOneResponse, EnableManualFanControlRequest,
    EnableManualFanControlResponse, FixedDutyRequest, FixedDutyResponse, HealthRequest,
    HealthResponse, InitializeDeviceRequest, InitializeDeviceResponse, LcdRequest, LcdResponse,
    LightingRequest, LightingResponse, ListDevicesRequest, ListDevicesResponse,
    ResetChannelRequest, ResetChannelResponse, ShutdownRequest, ShutdownResponse,
    SpeedProfileRequest, SpeedProfileResponse, StatusRequest, StatusResponse, health_response,
};
use crate::engine::Engine;
use crate::models::v1::{ChannelInfo, Device, DeviceInfo, DriverInfo, SpeedOptions, channel_info, status};
use crate::{SERVICE_ID, VERSION, models};
use log::debug;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;
use tonic::{Request, Response, Status};

pub const DEVICE_ID: &str = "omen_fans";
pub const CHANNELS: [(&str, &str); 2] = [("fan1", "CPU fan"), ("fan2", "GPU fan")];

pub struct OmenService {
    engine: Arc<Mutex<Engine>>,
    hwmon_dir: String,
    started: Instant,
}

impl OmenService {
    pub fn new(engine: Arc<Mutex<Engine>>, hwmon_dir: String) -> Self {
        Self { engine, hwmon_dir, started: Instant::now() }
    }

    fn engine(&self) -> MutexGuard<'_, Engine> {
        // A poisoned lock means a panic already ran the hook; keep answering safely.
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn feed(&self) {
        let mut e = self.engine();
        let now = e.now();
        e.ctrl.rpc(now);
    }

    fn device(&self) -> Device {
        let channels: HashMap<String, ChannelInfo> = CHANNELS
            .iter()
            .map(|(id, label)| {
                (
                    id.to_string(),
                    ChannelInfo {
                        label: Some(label.to_string()),
                        options: Some(channel_info::Options::SpeedOptions(SpeedOptions {
                            min_duty: 0,
                            max_duty: 100,
                            fixed_enabled: true,
                            extension: None,
                        })),
                    },
                )
            })
            .collect();
        Device {
            id: DEVICE_ID.to_string(),
            name: "HP OMEN fans".to_string(),
            uid_info: Some(format!("{SERVICE_ID}:{DEVICE_ID}")),
            info: Some(DeviceInfo {
                channels,
                temps: HashMap::new(),
                lighting_speeds: vec![],
                temp_min: None,
                temp_max: None,
                profile_min_length: None,
                profile_max_length: None,
                model: Some("HP OMEN fans (0 % = firmware auto, fans stop when cool)".to_string()),
                driver_info: Some(DriverInfo {
                    name: Some(SERVICE_ID.to_string()),
                    version: Some(VERSION.to_string()),
                    locations: vec![self.hwmon_dir.clone()],
                }),
            }),
        }
    }
}

fn check_device(id: &str) -> Result<(), Status> {
    if id == DEVICE_ID { Ok(()) } else { Err(Status::not_found("unknown device")) }
}

fn channel(id: &str) -> Result<usize, Status> {
    CHANNELS.iter().position(|(c, _)| *c == id).ok_or_else(|| Status::not_found("unknown channel"))
}

#[tonic::async_trait]
impl DeviceService for OmenService {
    async fn health(&self, _r: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
        self.feed();
        Ok(Response::new(HealthResponse {
            name: SERVICE_ID.to_string(),
            version: VERSION.to_string(),
            status: health_response::Status::Ok.into(),
            uptime_seconds: self.started.elapsed().as_secs(),
        }))
    }

    async fn list_devices(&self, _r: Request<ListDevicesRequest>) -> Result<Response<ListDevicesResponse>, Status> {
        self.feed();
        Ok(Response::new(ListDevicesResponse { devices: vec![self.device()] }))
    }

    /// Also called after resume: the next cycle writes the requested state again.
    async fn initialize_device(&self, r: Request<InitializeDeviceRequest>) -> Result<Response<InitializeDeviceResponse>, Status> {
        check_device(&r.get_ref().device_id)?;
        let mut e = self.engine();
        let now = e.now();
        e.ctrl.rpc(now);
        e.ctrl.failed(); // forget what we think we wrote; rewrite it
        e.cycle();
        Ok(Response::new(InitializeDeviceResponse {}))
    }

    async fn shutdown(&self, _r: Request<ShutdownRequest>) -> Result<Response<ShutdownResponse>, Status> {
        self.engine().stop();
        Ok(Response::new(ShutdownResponse {}))
    }

    async fn status(&self, r: Request<StatusRequest>) -> Result<Response<StatusResponse>, Status> {
        check_device(&r.get_ref().device_id)?;
        let e = self.engine();
        let mut e = e;
        let now = e.now();
        e.ctrl.rpc(now);
        let status = e
            .readings()
            .iter()
            .zip(CHANNELS)
            .map(|((rpm, duty), (id, _))| models::v1::Status {
                id: id.to_string(),
                metric: Some(status::Metric::Speed(status::FanSpeed { duty: *duty, rpm: *rpm })),
            })
            .collect();
        Ok(Response::new(StatusResponse { status }))
    }

    async fn reset_channel(&self, r: Request<ResetChannelRequest>) -> Result<Response<ResetChannelResponse>, Status> {
        let r = r.get_ref();
        check_device(&r.device_id)?;
        let ch = channel(&r.channel_id)?;
        let mut e = self.engine();
        let now = e.now();
        e.ctrl.reset(ch, now);
        debug!("ResetChannel {}", r.channel_id);
        e.cycle();
        Ok(Response::new(ResetChannelResponse {}))
    }

    async fn enable_manual_fan_control(
        &self,
        r: Request<EnableManualFanControlRequest>,
    ) -> Result<Response<EnableManualFanControlResponse>, Status> {
        let r = r.get_ref();
        check_device(&r.device_id)?;
        channel(&r.channel_id)?;
        // The mode follows the duties; nothing to switch here.
        self.feed();
        Ok(Response::new(EnableManualFanControlResponse {}))
    }

    async fn fixed_duty(&self, r: Request<FixedDutyRequest>) -> Result<Response<FixedDutyResponse>, Status> {
        let r = r.get_ref();
        check_device(&r.device_id)?;
        let ch = channel(&r.channel_id)?;
        let mut e = self.engine();
        let now = e.now();
        e.ctrl.set_duty(ch, r.duty, now);
        debug!("FixedDuty {} {} %", r.channel_id, r.duty);
        e.cycle();
        Ok(Response::new(FixedDutyResponse {}))
    }

    async fn speed_profile(&self, _r: Request<SpeedProfileRequest>) -> Result<Response<SpeedProfileResponse>, Status> {
        Err(Status::unimplemented("curves run in CoolerControl"))
    }

    async fn lighting(&self, _r: Request<LightingRequest>) -> Result<Response<LightingResponse>, Status> {
        Err(Status::unimplemented("no lighting"))
    }

    async fn lcd(&self, _r: Request<LcdRequest>) -> Result<Response<LcdResponse>, Status> {
        Err(Status::unimplemented("no LCD"))
    }

    async fn custom_function_one(&self, _r: Request<CustomFunctionOneRequest>) -> Result<Response<CustomFunctionOneResponse>, Status> {
        Err(Status::unimplemented("no custom function"))
    }
}
