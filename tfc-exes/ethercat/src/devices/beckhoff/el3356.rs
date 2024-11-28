use async_trait::async_trait;
use atomic_refcell::AtomicRefMut;
use ethercrab::{EtherCrabWireReadWrite, SubDevice, SubDevicePdi, SubDeviceRef};
use ethercrab_wire::{EtherCrabWireRead, EtherCrabWireWrite};
use log::{info, warn};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use tfc::confman::ConfMan;
use zbus::interface;

use crate::define_value_type;
use crate::devices::device_trait::{Device, DeviceInfo, Index, WriteValueIndex};

static RX_PDO_ASSIGN: u16 = 0x1C12;
static TX_PDO_ASSIGN: u16 = 0x1C13;
static RX_PDO_MAPPING: u16 = 0x1600;
static TX_PDO_MAPPING: u16 = 0x1A00;

smlang::statemachine! {
    name: Calibrate,
    derive_states: [Debug, Clone],
    derive_events: [Debug, Clone],
    transitions: {
        *Idle + SetZeroCalibration = ZeroCalibration,
        ZeroCalibration + SetCalibration = Calibration,
        Calibration + SetIdle = Idle,
    }
}

#[derive(Debug, EtherCrabWireRead)]
#[wire(bytes = 2)]
struct StatusWord {
    #[wire(bits = 1, pre_skip = 1)]
    over_range: bool,
    #[wire(bits = 1, pre_skip = 1)]
    data_invalid: bool,
    #[wire(bits = 1, pre_skip = 2)]
    collective_error: bool,
    #[wire(bits = 1)]
    calibration_in_progress: bool,
    #[wire(bits = 1)]
    /// Steady state (Idling recognition)
    /// If the load value remains within a range of values y for longer than time x, then the SteadyState is
    /// activated in the StatusWord.
    /// TODO: The parameters x and y can be specified in the CoE
    steady_state: bool,
    #[wire(bits = 1, pre_skip = 4)]
    /// Synchronization error
    sync_error: bool,
    #[wire(bits = 1, pre_skip = 1)]
    /// toggeles 0->1->0 with each updated data set
    tx_pdo: bool,
}

#[derive(Debug, EtherCrabWireWrite)]
#[wire(bytes = 2)]
struct ControlWord {
    #[wire(bits = 1)]
    /// Starts the self-calibration immediately
    start_calibration: bool,
    #[wire(bits = 1)]
    /// The measuring amplifiers are periodically subjected to examination and self-calibration. Several analog
    /// switches are provided for this purpose, so that the various calibration signals can be connected. It is
    /// important for this process that the entire signal path, including all passive components, is examined at every
    /// phase of the calibration. Only the interference suppression elements (L/C combination) and the analog
    /// switches themselves cannot be examined. In addition, a self-test is carried out at longer intervals.
    /// The self-calibration is carried out every three minutes in the default setting.
    /// Self-calibration
    /// The time interval is set in 100 ms steps with object 0x8000:31 [} 174]; default: 3 min.
    /// Duration approx. 150 ms
    /// Self-test
    /// is additional carried out together with every nth self-calibration.
    /// The multiple (n) is set with object 0x8000:32 [} 174]; default: 10
    /// additional duration approx. 70 ms.
    disable_calibration: bool,
    #[wire(bits = 1)]
    /// If the terminal is placed in the freeze state by InputFreeze in the control word, no further analog measured
    /// values are relayed to the internal filter. This function is usable, for example, if a filling surge is expected from
    /// the application that would unnecessarily overdrive the filters due to the force load. This would result in a
    /// certain amount of time elapsing until the filter had settled again. The user himself must determine a sensible
    /// InputFreeze time for his filling procedure.
    input_freeze: bool,
    #[wire(bits = 1)]
    /// Mode 0: High precision Analog conversion at 10.5 kSps (samples per second) Slow conversion and thus high accuracy
    /// Typical Latency 7.2 ms
    /// Mode 1: High speed / low latency Analog conversion at 105.5 kSps (samples per second) Fast conversion with low latency
    /// Typical Latency 0.72 ms
    sample_mode: bool,
    #[wire(bits = 1, post_skip = 11)]
    /// When taring, the scales are set to zero using an arbitrary applied load; i.e. an offset correction is performed.
    /// The EL3356 supports two tarings; it is recommended to set a strong filter when taring.
    /// Temporary tare: The correction value is NOT stored in the terminal and is lost in the event of a power failure.
    /// Permanent tare: The correction value is stored locally in the terminal's EEPROM and is not lost in the event of a power
    /// failure.
    tare: bool,
}

#[derive(Debug, EtherCrabWireRead)]
#[wire(bytes = 6)]
struct InputPdo {
    #[wire(bits = 16)]
    status_word: StatusWord,
    #[wire(bits = 32)]
    raw_value: f32,
}

#[derive(Debug, EtherCrabWireWrite)]
#[wire(bytes = 2)]
struct OutputPdo {
    #[wire(bits = 16)]
    control_word: ControlWord,
}

#[derive(Debug, EtherCrabWireReadWrite, Copy, Clone, Serialize, Deserialize, JsonSchema)]
#[repr(u16)]
enum Filter {
    FIR50 = 0,
    FIR60 = 1,
    IIR1 = 2,
    IIR2 = 3,
    IIR3 = 4,
    IIR4 = 5,
    IIR5 = 6,
    IIR6 = 7,
    IIR7 = 8,
    IIR8 = 9,
    DynamicIIR = 10,
    PDOFilterFrequency = 11,
}
impl Default for Filter {
    fn default() -> Self {
        Self::FIR50
    }
}
impl Index for Filter {
    const INDEX: u16 = 0x8000;
    const SUBINDEX: u8 = 0x11;
}

// From beckhoff manual, please note that the weigher can itself output unit (kg) or you can neutralize this by configuring the relevant parameters
// 5.5.6 Calculating the weight
// Each measurement of the analog inputs is followed by the calculation of the resulting weight or the resulting
// force, which is made up of the ratio of the measuring signal to the reference signal and of several
// calibrations.
// Y_R = (U_Diff / U_Ref) ⋅ A_i                     (1.0) Calculation of the raw value in mV/V
// Y_L = ( (Y_R - C_ZB) / (C_n - C_ZB) ) ⋅ Emax     (1.1) Calculation of the weight
// Y_S = Y_L ⋅ A_S                                  (1.2) Scaling factor (e.g. factor 1000 for rescaling from kg to g)
// Y_G = Y_S ⋅ (G / 9.80665)                        (1.3) Influence of acceleration of gravity
// Y_AUS = Y_G ⋅ Gain - Tare                        (1.4) Gain and Tare
// | Name  | Description | CoE Index |
// |-------|-------------|-----------|
// | UDiff | Bridge voltage/differential voltage of the sensor element, after averager and filter | |
// | URef  | Bridge supply voltage/reference signal of the sensor element, after averager and filter | |
// | Ai    | Internal gain, not changeable. This factor accounts for the unit standardization from mV to V and the different full-scale deflections of the input channels | |
// | Cn    | Nominal characteristic value of the sensor element (unit mV/V, e.g. nominally 2 mV/V or 2.0234 mV/V according to calibration protocol) | 0x8000:23 |
// | CZB   | Zero balance of the sensor element (unit mV/V, e.g. -0.0142 according to calibration protocol) | 0x8000:25 |
// | Emax  | Nominal load of the sensor element. The firmware always calculates without units, the unit (kg, g, lb) used here is then also applicable to the result | 0x8000:24 |
// | AS    | Scaling factor (e.g. factor 1000 for rescaling from kg to g) | 0x8000:27 |
// | G     | Acceleration of gravity in m/s² (default: 9.80665 m/s²) | 0x8000:26 |
// | Gain  | | 0x8000:21 |
// | Tare  | | 0x8000:22 |

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub enum Mode {
    Scale = 0,
    Reference = 1,
}
impl Default for Mode {
    fn default() -> Self {
        Self::Scale
    }
}

define_value_type!(NominalValue, f32, 1.0, 0x8000, 0x23);
define_value_type!(Gravity, f32, 9.806650, 0x8000, 0x26);
define_value_type!(ZeroBalance, f32, 0.0, 0x8000, 0x25);
define_value_type!(ScaleFactor, f32, 1000.0, 0x8000, 0x27);

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
struct Config {
    filter: Filter,
    nominal_load: f32, // kg
    #[schemars(description = "Mass of reference load")]
    calibration_load: f64, // kg
    #[schemars(
        description = "Nominal characteristic value of the sensor element, Set to 1 if you wan't raw value from load cell. mV/V"
    )]
    nominal_value: Option<NominalValue>,
    #[schemars(
        description = "Gravity of earth, default: None is 9.80665 m/s^2. Set to 1 if you want raw value from load cell"
    )]
    gravity: Option<Gravity>,
    #[schemars(
        description = "Zero balance of the sensor element. mV/V. Set to 0 if you want raw value from load cell"
    )]
    zero_balance: Option<ZeroBalance>,
    #[schemars(
        description = "This factor can be used to re-scale the process data. In order to change the display from kg to g, for example, the factor 1000 can be entered here."
    )]
    scale_factor: Option<ScaleFactor>,
    // skip deserializing does not work, it needs to be read only in schema, since we need to read the value from file
    // #[serde(skip_deserializing)] // read only
    #[schemars(description = "Signal read from load cell at zero calibration")]
    zero_signal_read: f64,
    // #[serde(skip_deserializing)] // read only
    #[schemars(description = "Signal read from load cell at calibration with calibration load")]
    calibration_signal_read: f64,
    #[schemars(description = "Resolution, smallest scale increment, like 0.001 kg or 0.010 kg")]
    resolution: f64, // kg
    #[schemars(description = "Mode of operation")]
    mode: Mode,
    #[schemars(
        description = "Number of samples to use for average filter. Process data interval."
    )]
    filter_window: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            filter: Filter::default(),
            nominal_load: 5.0,
            calibration_load: 5.0,
            nominal_value: None,
            gravity: None,
            zero_balance: None,
            scale_factor: None,
            zero_signal_read: 0.0,
            calibration_signal_read: 1000.0,
            resolution: 0.001,
            mode: Mode::default(),
            filter_window: 100,
        }
    }
}

pub struct DbusInterface {}
#[interface(name = "is.centroid.el3356")]
impl DbusInterface {
    async fn set_zero(&self) -> Result<(), zbus::fdo::Error> {
        Ok(())
    }
}

// from https://github.com/rust-lang/rust/issues/72353#issuecomment-1093729062
pub struct AtomicF64 {
    storage: std::sync::atomic::AtomicU64,
}
impl AtomicF64 {
    pub fn new(value: f64) -> Self {
        let as_u64 = value.to_bits();
        Self {
            storage: std::sync::atomic::AtomicU64::new(as_u64),
        }
    }
    pub fn store(&self, value: f64, ordering: std::sync::atomic::Ordering) {
        let as_u64 = value.to_bits();
        self.storage.store(as_u64, ordering)
    }
    pub fn load(&self, ordering: std::sync::atomic::Ordering) -> f64 {
        let as_u64 = self.storage.load(ordering);
        f64::from_bits(as_u64)
    }
}

pub struct Scale {
    tare_slot: tfc::ipc::Slot<bool>,
    ratio_slot: tfc::ipc::Slot<f64>,
    mass_signal: tfc::ipc::Signal<f64>,
    tare: f64, // signal read tare for fixing zero point, runtime value
    ratio: Arc<AtomicF64>,
    tare_cmd: Arc<std::sync::atomic::AtomicBool>,
    last_mass: f64,
}
impl Scale {
    pub fn new(dbus: zbus::Connection, prefix: String) -> Self {
        let mut ratio_slot = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(
                format!("{prefix}_ratio").as_str(),
                Some("Scale raw input by this factor, useful for onboard vessel scale"),
            ),
        );
        let ratio = Arc::new(AtomicF64::new(1.0));
        let ratio_cp = ratio.clone();
        ratio_slot.recv(Box::new(move |new_ratio| {
            ratio_cp.store(*new_ratio, std::sync::atomic::Ordering::Relaxed);
        }));
        let mass_signal = tfc::ipc::Signal::new(
            dbus.clone(),
            tfc::ipc::Base::new(format!("{prefix}_mass").as_str(), Some("Mass output in kg")),
        );
        tfc::ipc::dbus::SignalInterface::register(
            mass_signal.base(),
            dbus.clone(),
            mass_signal.subscribe(),
        );
        // tare slot
        let mut tare_slot = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(
                format!("{prefix}_tare").as_str(),
                Some("Offset current weight on cell as tare, meaning it will zero out the current weight"),
            ),
        );
        tfc::ipc::dbus::SlotInterface::register(
            tare_slot.base(),
            dbus.clone(),
            tare_slot.channel("dbus"),
        );
        let tare = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tare_cp = tare.clone();
        tare_slot.recv(Box::new(move |new_tare| {
            tare_cp.store(*new_tare, std::sync::atomic::Ordering::Relaxed);
        }));

        Self {
            tare_slot,
            ratio_slot,
            mass_signal,
            tare: 0.0,
            ratio,
            last_mass: 0.0,
            tare_cmd: tare,
        }
    }
}

pub struct ReferenceScale {
    ratio_signal: tfc::ipc::Signal<f64>,
    last_ratio: f64,
    ratio_epsilon: f64, // represent resolution of 0.001 or whatever it is set to
}

impl ReferenceScale {
    pub fn new(dbus: zbus::Connection, prefix: String) -> Self {
        let ratio_signal = tfc::ipc::Signal::new(
            dbus.clone(),
            tfc::ipc::Base::new(
                format!("{prefix}_ratio").as_str(),
                Some("Output ratio of calibration load compared to current load"),
            ),
        );
        tfc::ipc::dbus::SignalInterface::register(
            ratio_signal.base(),
            dbus.clone(),
            ratio_signal.subscribe(),
        );
        Self {
            ratio_signal,
            last_ratio: 1.0,
            ratio_epsilon: 0.001,
        }
    }
}

pub enum ModeImpl {
    Scale(Scale),
    Reference(ReferenceScale),
}

mod my_avg {
    // TODO: I would much rather like to use median filter, but is is harder to implement it REALLY FAST than average
    pub struct AvgFilter {
        window: ringbuf::HeapRb<f64>,
        sum: f64,
    }
    impl AvgFilter {
        pub fn new(capacity: usize) -> Self {
            Self {
                window: ringbuf::HeapRb::new(capacity),
                sum: 0.0,
            }
        }
        pub fn consume(&mut self, value: f64) -> f64 {
            use ringbuf::traits::{Consumer, Observer, Producer};
            if self.window.is_full() {
                let old_value = self.window.try_pop().expect("This should never happen");
                self.sum -= old_value;
            }
            self.sum += value;
            self.window
                .try_push(value)
                .expect("This should never happen");
            // this is incorrect while window is getting filled, should not be problematic
            self.sum / self.window.capacity().get() as f64
        }
    }
}

pub struct El3356 {
    cnt: u128,
    config: ConfMan<Config>,
    log_key: String,
    mode: ModeImpl,
    filter: my_avg::AvgFilter,
    calibrate_cmd: Arc<std::sync::atomic::AtomicBool>,
    zero_calibrate_cmd: Arc<std::sync::atomic::AtomicBool>,
    calibrate_slot: tfc::ipc::Slot<bool>,
    zero_calibrate_slot: tfc::ipc::Slot<bool>,
}

impl El3356 {
    pub fn new(dbus: zbus::Connection, slave_number: u16, alias_address: u16) -> Self {
        let mut prefix = format!("el3356_{slave_number}");
        if alias_address != 0 {
            prefix = format!("el3356_alias_{alias_address}");
        }
        let config: ConfMan<Config> = ConfMan::new(dbus.clone(), &prefix);

        let mode = match config.read().mode {
            Mode::Scale => ModeImpl::Scale(Scale::new(dbus.clone(), prefix.clone())),
            Mode::Reference => {
                ModeImpl::Reference(ReferenceScale::new(dbus.clone(), prefix.clone()))
            }
        };
        let filter = my_avg::AvgFilter::new(config.read().filter_window as usize);

        let mut calibrate_slot = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(
                format!("{prefix}_calibrate").as_str(),
                Some("Take current weight as calibration point, according to calibration load in config"),
            ),
        );
        tfc::ipc::dbus::SlotInterface::register(
            calibrate_slot.base(),
            dbus.clone(),
            calibrate_slot.channel("dbus"),
        );
        let calibrate_cmd = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let calibrate_cmd_cp = calibrate_cmd.clone();
        calibrate_slot.recv(Box::new(move |new_calibrate| {
            calibrate_cmd_cp.store(*new_calibrate, std::sync::atomic::Ordering::Relaxed);
        }));
        // zero calibrate slot
        let mut zero_calibrate_slot = tfc::ipc::Slot::new(
            dbus.clone(),
            tfc::ipc::Base::new(
                format!("{prefix}_zero_calibrate").as_str(),
                Some("Offset current weight on cell as zero"),
            ),
        );
        tfc::ipc::dbus::SlotInterface::register(
            zero_calibrate_slot.base(),
            dbus.clone(),
            zero_calibrate_slot.channel("dbus"),
        );
        let zero_calibrate_cmd = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let zero_calibrate_cmd_cp = zero_calibrate_cmd.clone();
        zero_calibrate_slot.recv(Box::new(move |new_zero_calibrate| {
            zero_calibrate_cmd_cp.store(*new_zero_calibrate, std::sync::atomic::Ordering::Relaxed);
        }));

        Self {
            cnt: 0,
            config,
            log_key: prefix.clone(),
            mode,
            filter,
            calibrate_cmd,
            zero_calibrate_cmd,
            calibrate_slot,
            zero_calibrate_slot,
        }
    }
    /// Zero calibration
    /// Resets the unit to default parameters and than records internally the current weight and sets as zero
    pub async fn zero_calibrate<S: std::ops::Deref<Target = SubDevice>>(
        device: &mut SubDeviceRef<'_, S>,
        nominal_load: f32,
    ) -> Result<(), ethercrab::error::Error> {
        // 1. Perform a CoE reset with object 0x1011:01 see Restoring the delivery state [} 206
        // If this object is set to “0x64616F6C” in the set value dialog, all backup objects are reset to their delivery state.
        device.sdo_write(0x1011, 0x01, 0x64616F6C as u32).await?;
        // 2. Activate mode 0 via the control word (EL3356-0010 only)
        // JBB NOTE: We only use mode 0 which is high precision
        // 3. Set scale factor to 1 (0x8000:27 [} 174])
        device.sdo_write(0x8000, 0x27, 1 as f32).await?;
        // 4. Set gravity of earth (0x8000:26) [} 174] if necessary (default: 9.806650)
        // JBB NOTE: We don't need to change this maybe later
        // 5. Set gain to (0x8000:21 [} 174]) = 1
        device.sdo_write(0x8000, 0x21, 1 as f32).await?;
        // 6. Set tare to 0 (0x8000:22 [} 174])
        device.sdo_write(0x8000, 0x22, 0 as f32).await?;
        // 7. Set the filter (0x8000:11 [} 174]) to the strongest level: IIR8
        device.sdo_write_value_index(Filter::IIR8).await?;
        // 8. Specify the nominal load of the sensor in 0x8000:24 [} 174] (“Nominal load”)
        // JBB NOTE: I disagree with this, why is it needed to know nominal load? But let's do it
        device.sdo_write(0x8000, 0x24, nominal_load).await?;
        // 9. Zero balance: Do not load the scales
        // As soon as the measured value indicates a constant value for at least 10 seconds, execute the
        // command “0x0101” (257dec) on CoE object 0xFB00:01 [} 176].
        // This command causes the current mV/V value (0x9000:11 [} 177]) to be entered in the “Zero balance” object.
        // Check: CoE objects 0xFB00:02 and 0xFB00:03 must contain “0” after execution.
        device.sdo_write(0xFB00, 0x01, 0x0101 as u16).await?; // todo this is of type OCTET - STRING[2] ?
        loop {
            let status: u8 = device.sdo_read(0xFB00, 0x02).await?;
            let response: u32 = device.sdo_read(0xFB00, 0x03).await?;
            if status == 0 && response == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        Ok(())
    }
    /// Calibrate
    /// following zero calibrate a calibrate is necessary, it sets the second point of linear interpolation of the signal to the measured unit
    pub async fn calibrate<S: std::ops::Deref<Target = SubDevice>>(
        device: &mut SubDeviceRef<'_, S>,
        calibration_load: f32,
        filter: Filter,
    ) -> Result<(), ethercrab::error::Error> {
        // 10. Load the scales with a reference load. This should be at least 20% of the rated load. The larger the
        // reference load, the better the sensor values can be calculated.
        // In object 0x8000:28 [} 174] (“Reference load”), enter the load in the same unit as the rated load (0x8000:24 [} 174]).
        // As soon as the measured value indicates a constant value for at least 10 seconds, execute the
        // command “0x0102” (258dec) on CoE object 0xFB00:01 [} 176].
        // By means of this command the EL3356 determines the output value for the nominal weight (“Rated output”)
        // Check: CoE objects 0xFB00:02 and 0xFB00:03 must contain “0” after execution.
        device.sdo_write(0x8000, 0x28, calibration_load).await?;
        device.sdo_write(0xFB00, 0x01, 0x0102 as u16).await?; // todo this is of type OCTET - STRING[2] ?
        loop {
            let status: u8 = device.sdo_read(0xFB00, 0x02).await?;
            let response: u32 = device.sdo_read(0xFB00, 0x03).await?;
            if status == 0 && response == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        // 11. Reset: execute the command “0x0000” (0dec) on CoE object 0xFB00:01 [} 176].
        device.sdo_write(0xFB00, 0x01, 0x0000 as u16).await?;
        // 12. Set the filter to a lower stage.
        device.sdo_write_value_index(filter).await?;
        Ok(())
    }
}

#[async_trait]
impl Device for El3356 {
    async fn setup<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, AtomicRefMut<'group, SubDevice>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(nominal_value) = self.config.read().nominal_value {
            device.sdo_write_value_index(nominal_value).await?;
        }
        if let Some(gravity) = self.config.read().gravity {
            device.sdo_write_value_index(gravity).await?;
        }
        if let Some(zero_balance) = self.config.read().zero_balance {
            device.sdo_write_value_index(zero_balance).await?;
        }
        if let Some(scale_factor) = self.config.read().scale_factor {
            device.sdo_write_value_index(scale_factor).await?;
        }

        device.sdo_write(0x8000, 0x27, 1000000.0 as f32).await?;

        device.sdo_write(TX_PDO_ASSIGN, 0x00, 0 as u8).await?;
        device.sdo_write(TX_PDO_ASSIGN, 0x01, 0x1A00 as u16).await?;
        device.sdo_write(TX_PDO_ASSIGN, 0x02, 0x1A02 as u16).await?; // use REAL from 0x1A02 pdo mapping

        // device.sdo_write(TX_PDO_ASSIGN, 0x02, 0x1A01 as u16).await?; // use int from 0x1A01pdo mapping
        device.sdo_write(TX_PDO_ASSIGN, 0x00, 0x02 as u8).await?;

        Ok(())
    }
    async fn process_data<'maindevice, 'group>(
        &mut self,
        device: &mut SubDeviceRef<'maindevice, SubDevicePdi<'group>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cnt += 1;

        let (i, mut o) = device.io_raw_mut();

        let input_pdo = InputPdo::unpack_from_slice(&i)?;

        // tx_pdo does not change when the value from the sensor is the same for some period of time
        // the average filter needs to include all this period so let's not return early here
        // if !input_pdo.status_word.tx_pdo {
        //     return Ok(());
        // }

        // debug logging
        // if self.cnt % 1000 == 0 {
        //     let input_pdo = InputPdo::unpack_from_slice(&i).expect("Error unpacking input PDO");
        //     let output_pdo = OutputPdo {
        //         control_word: ControlWord {
        //             start_calibration: false,
        //             disable_calibration: true,
        //             input_freeze: false,
        //             sample_mode: false,
        //             tare: false,
        //         },
        //     };
        //     warn!(target: &self.log_key, "El3356: {}, i: {input_pdo:?}, o: {o:?}", self.cnt);
        //     output_pdo
        //         .pack_to_slice(&mut o)
        //         .expect("Error packing output PDO");
        // }
        let raw_signal = input_pdo.raw_value as f64;

        let signal = self.filter.consume(raw_signal);

        let signal = match &self.mode {
            ModeImpl::Scale(ref scale) => {
                let ratio = scale.ratio.load(std::sync::atomic::Ordering::Relaxed);
                signal / ratio // normalized value of raw value with respect to the given ratio
            }
            ModeImpl::Reference(_) => signal,
        };

        // let's see whether we should set zero signal read now
        if self
            .zero_calibrate_cmd
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.config.write().value_mut().zero_signal_read = signal;
            info!(target: &self.log_key, "Zero signal read set to {}", signal);
            self.zero_calibrate_cmd
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }

        // let's see whether we should set calibration signal read now
        if self
            .calibrate_cmd
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.config.write().value_mut().calibration_signal_read = signal;
            info!(target: &self.log_key, "Calibration signal read set to {}", signal);
            self.calibrate_cmd
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }

        let zero = match &self.mode {
            ModeImpl::Scale(ref scale) => self.config.read().zero_signal_read + scale.tare,
            ModeImpl::Reference(_) => self.config.read().zero_signal_read,
        };

        let signal = signal - zero; // we are now offsetted by zero reading

        let calibration_signal = self.config.read().calibration_signal_read;
        let zeroed_calibration_signal = calibration_signal - zero;
        if zeroed_calibration_signal <= 0.0 {
            return Err("Zeroed calibration signal is <= 0, this should not happen".into());
        }
        // now we have the mass in full resolution
        let signal_mass = signal * self.config.read().calibration_load / zeroed_calibration_signal;

        match &mut self.mode {
            ModeImpl::Scale(ref mut scale) => {
                // let's see whether we should set tare signal read now
                // BIG TODO: we need to make sure that the tare offset is not too large, have configurable limits
                if scale.tare_cmd.load(std::sync::atomic::Ordering::Relaxed) {
                    scale.tare += signal;
                    info!(target: &self.log_key, "Tare offset set to {}", scale.tare);
                    scale
                        .tare_cmd
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                }

                // lets scale the full resolution down to the given resolution
                let scaling_factor = 1.0 / self.config.read().resolution; // 0.001 example
                let signal_mass_rounded = (signal_mass * scaling_factor).round() / scaling_factor;

                if signal_mass_rounded.is_nan() {
                    return Err("Signal mass rounded is NaN, this should not happen".into());
                }
                if signal_mass_rounded.is_infinite() {
                    return Err("Signal mass rounded is infinite, this should not happen".into());
                }

                // Now we have a valid measurement let's process it

                if signal_mass_rounded == scale.last_mass {
                    return Ok(());
                }
                scale.last_mass = signal_mass_rounded;
                scale.mass_signal.async_send(signal_mass_rounded).await?;
                info!(target: &self.log_key, "Mass signal sent: {} at raw signal {}", signal_mass_rounded, raw_signal);
                Ok(()) as Result<(), Box<dyn Error + Send + Sync>>
            }
            ModeImpl::Reference(ref mut reference) => {
                let ref_ratio = signal_mass / self.config.read().calibration_load;
                // info!(target: &self.log_key, "Reference ratio: {} at signal {}", ref_ratio, signal);

                if (ref_ratio - reference.last_ratio).abs() > reference.ratio_epsilon {
                    if ref_ratio > 0.0 && ref_ratio < 2.0 {
                        reference.ratio_signal.async_send(ref_ratio).await?;
                        reference.last_ratio = ref_ratio;
                    } else {
                        return Err(format!(
                            "Reference ratio is out of bounds: {}, this should not happen",
                            ref_ratio
                        )
                        .into());
                    }
                }

                Ok(())
            }
        }?;

        Ok(())
    }
}

impl DeviceInfo for El3356 {
    const VENDOR_ID: u32 = 0x2;
    const PRODUCT_ID: u32 = 0x0d1c3052;
    const NAME: &'static str = "El3356";
}
