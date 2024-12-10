#[cfg(feature = "opcua-expose")]
use crate::opcua::OpcuaServerHandle;
use ethercrab::{
    std::{ethercat_now, tx_rx_task},
    MainDevice, MainDeviceConfig, PduStorage, SubDeviceGroup, Timeouts,
};
use log::{debug, error, info, trace, warn};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::Instant;
use std::{sync::Arc, time::Duration};
use tfc::confman::ConfMan;
use tfc::time::MicroDuration;
use zbus;
use zbus::Connection;

use crate::devices::device::make_device;
use crate::devices::device_trait::{Device, UnimplementedDevice};

/// Maximum number of SubDevices that can be stored. This must be a power of 2 greater than 1.
const MAX_SUBDEVICES: usize = 16;
/// Maximum PDU data payload size - set this to the max PDI size or higher.
const MAX_PDU_DATA: usize = 1100;
/// Maximum number of EtherCAT frames that can be in flight at any one time.
const MAX_FRAMES: usize = 16;
/// Maximum total PDI length. // LENZE i550 requires 66 bytes
const PDI_LEN: usize = 76;

static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();

#[derive(Deserialize, Serialize, JsonSchema)]
struct BusConfig {
    pub interface: String,
    pub cycle_time: MicroDuration,
    #[schemars(
        description = "Minimum number of subdevices that must be in the init state before the bus is transitioned into operational"
    )]
    pub subdevice_min_count: u8,
}
impl Default for BusConfig {
    fn default() -> Self {
        Self {
            interface: "eth0".to_string(),
            cycle_time: Duration::from_millis(1).into(),
            subdevice_min_count: 1,
        }
    }
}

pub struct Bus {
    main_device: Arc<MainDevice<'static>>,
    config: ConfMan<BusConfig>,
    devices: [Box<dyn Device + Send + Sync>; MAX_SUBDEVICES],
    group: Option<SubDeviceGroup<MAX_SUBDEVICES, PDI_LEN, ethercrab::subdevice_group::Op>>,
    log_key: String,
    expected_working_counter: u16,
    #[cfg(feature = "opcua-expose")]
    opcua_handle: OpcuaServerHandle,
}

impl Bus {
    pub fn new(
        conn: Connection,
        #[cfg(feature = "opcua-expose")] opcua_handle: OpcuaServerHandle,
    ) -> Self {
        let (tx, rx, pdu_loop) = PDU_STORAGE.try_split().expect("can only split once");

        let main_device = Arc::new(MainDevice::new(
            pdu_loop,
            Timeouts::default(),
            MainDeviceConfig::default(),
        ));

        let config = ConfMan::<BusConfig>::new(conn.clone(), "bus");
        tokio::spawn(tx_rx_task(&config.read().interface, tx, rx).expect(
            "spawn TX/RX task failed on interface: {}",
            config.read().interface,
        ));
        Self {
            main_device,
            config,
            devices: std::array::from_fn(|_| {
                Box::new(UnimplementedDevice) as Box<dyn Device + Send + Sync>
            }),
            group: None,
            log_key: "ethercat".to_string(),
            expected_working_counter: 0,
            #[cfg(feature = "opcua-expose")]
            opcua_handle,
        }
    }
    pub async fn init(
        &mut self,
        dbus: zbus::Connection,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        debug!(target: &self.log_key, "Initializing main device");
        let mut group = self
            .main_device
            .init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now)
            .await?; // BIG TODO HOW CAN I CONFIGURE THIS TIMEOUT, that is putting group into init state? state_transition does not work here
        debug!(target: &self.log_key, "Initialized main device");

        if group.len() < self.config.read().subdevice_min_count as usize {
            return Err(format!(
                "Not enough subdevices in init state, expected: {}, got: {}",
                self.config.read().subdevice_min_count,
                group.len()
            )
            .into());
        }

        let mut index: u16 = 0;
        for (idx, mut subdevice) in group.iter(&self.main_device).enumerate() {
            let identity = subdevice.identity();
            if self.devices[idx].vendor_id() != identity.vendor_id
                || self.devices[idx].product_id() != identity.product_id
            {
                self.devices[idx] = make_device(
                    dbus.clone(),
                    identity.vendor_id,
                    identity.product_id,
                    index,
                    subdevice.alias_address(),
                    subdevice.name(),
                );
                #[cfg(feature = "opcua-expose")]
                self.devices[idx].opcua_register(
                    self.opcua_handle.manager.clone(),
                    self.opcua_handle.subscriptions.clone(),
                    self.opcua_handle.namespace,
                )?;
            }
            // TODO: Make futures that can be awaited in parallel
            self.devices[idx].setup(&mut subdevice).await.map_err(|e| {
                warn!(target: &self.log_key, "Failed to setup device {}: {}", index, e);
                e
            })?;
            index += 1;
        }
        trace!(target: &self.log_key, "Setup complete for devices: {}", index);

        // let group = group.into_op(&self.main_device).await?;

        let group = group.into_safe_op(&self.main_device).await?;

        debug!(target: &self.log_key, "Group in safe op");

        self.expected_working_counter = group.tx_rx(&self.main_device).await?;
        info!(target: &self.log_key, "Group in safe op Tx/Rx complete, now will expect working counter to be: {}", self.expected_working_counter);

        let group = group.into_op(&self.main_device).await?;

        debug!(target: &self.log_key, "Group in operational");

        self.group = Some(group);
        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ref mut group = self.group.as_mut().expect("Group not initialized");

        let mut tick_interval = tokio::time::interval(self.config.read().cycle_time.into());
        info!(target: &self.log_key, "Ethercat tick interval: {:?}", self.config.read().cycle_time);
        tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut cnt = 0;
        let mut instant = Instant::now();
        let mut tx_rx_duration = Duration::ZERO;
        let mut process_data_duration = Duration::ZERO;
        let mut device_errors: [Option<Box<dyn Error + Send + Sync>>; MAX_SUBDEVICES] =
            std::array::from_fn(|_| None);
        loop {
            let tx_rx_instant = Instant::now();
            let wc = group.tx_rx(&self.main_device).await?;
            tx_rx_duration += tx_rx_instant.elapsed();

            if wc != self.expected_working_counter {
                // TODO we need to recover less tremeendously than this
                // https://github.com/ethercrab-rs/ethercrab/discussions/253
                return Err(format!(
                    "Working counter mismatch, expected: {}, got: {}",
                    self.expected_working_counter, wc
                )
                .into());
            }

            let process_data_instant = Instant::now();
            for (device_index, mut subdevice) in group.iter(&self.main_device).enumerate() {
                if let Some(device) = self.devices.get_mut(device_index) {
                    match device.process_data(&mut subdevice).await {
                        Ok(()) => {
                            device_errors[device_index] = None;
                        }
                        Err(e) => {
                            if device_errors[device_index].is_none() {
                                warn!(target: &self.log_key, "Failed to process data for subdevice {}: {}", device_index, e);
                            }
                            device_errors[device_index] = Some(e);
                        }
                    }
                }
            }
            process_data_duration += process_data_instant.elapsed();

            tick_interval.tick().await;
            cnt += 1;
            if cnt % 1000 == 0 {
                info!(target: &self.log_key, "Ethercat tick interval: {:?}", instant.elapsed()/1000);
                info!(target: &self.log_key, "Tx/Rx duration: {:?}", tx_rx_duration/1000);
                info!(target: &self.log_key, "Process data duration: {:?}", process_data_duration/1000);
                instant = Instant::now();
                tx_rx_duration = Duration::ZERO;
                process_data_duration = Duration::ZERO;
            }
        }
        // If I comment the loop out the load falls down to 0 ish %
        // std::future::pending::<()>().await;

        // Ok(())
    }

    pub async fn init_and_run(
        &mut self,
        dbus: zbus::Connection,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        loop {
            loop {
                let res = self.init(dbus.clone()).await;
                if let Err(e) = res {
                    warn!(target: &self.log_key, "Failed to init: {}", e);
                } else {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }

            let _ = self.run().await.map_err(|e| {
                error!(target: &self.log_key, "Failed to run will retry: {}", e);
                e
            });
        }
    }
}
