//! A Matter multi-zone sprinkler controller emulator.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::cell::RefCell;

use libertas::{
    InlineByteBuffer, LIBERTAS_BROADCAST_DEST, LibertasDevice, LibertasVirtualDevice,
    NotificationArgument, libertas_data_read, libertas_data_write, libertas_device_send_response,
    libertas_get_sys_ticks, libertas_register_device_listener, libertas_timer_cancel,
    libertas_timer_new_interval, libertas_timer_update_interval,
};
use libertas_macros::{
    LibertasAvroDecode, LibertasAvroEncode, LibertasExport, libertas_data_schema,
    libertas_string_resources,
};
use libertas_matter::{
    IMStatusCode, MatterAttribute, MatterDevice, MatterReadCluster, MatterRequestContext,
    consts::{
        attributes::ValveConfigurationandControl as valve_attributes, clusters,
        commands::ValveConfigurationandControl as valve_commands,
    },
    decode_command,
    definitions::ValveConfigurationandControl::{
        attributes::{
            CurrentState, DefaultOpenDuration, OpenDuration, RemainingDuration, TargetState,
        },
        commands::{Close, Open},
        types::ValveStateEnum,
    },
    error::Error,
    frame::{self, Operation, PROTOCOL_MATTER, Status},
    tlv::{Element, FromTLV, Nullable, TLVWrite, Tag, ValueType},
};

const DEFAULT_OPEN_DURATION_SECONDS: u32 = 10 * 60;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const DEFAULT_OPEN_DURATION_RESOURCE: &str = "DEFAULT_OPEN_DURATION";

pub static APP_STRINGS: [(&str, &str); 1] = [(
    DEFAULT_OPEN_DURATION_RESOURCE,
    "Default open duration for %1$s.",
)];

/// Persistent valve data.
///
/// Every data shape written to the Libertas database is defined inline in
/// this union.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum ValveData {
    /// Matter DefaultOpenDuration attribute.
    DefaultOpenDuration {
        /// DefaultOpenDuration, in seconds.
        value: u32,
    },
}

struct Valve {
    device: LibertasDevice,
    is_open: bool,
    default_open_duration: u32,
    open_duration: Option<u32>,
    expiration_ticks: Option<u64>,
    timer: Option<u32>,
}

struct IrrigationContext {
    valves: Vec<Rc<RefCell<Valve>>>,
}

struct ValveContext {
    valve: Rc<RefCell<Valve>>,
    shared: Rc<RefCell<IrrigationContext>>,
}

struct TimerContext {
    valve: Rc<RefCell<Valve>>,
}

#[derive(Clone, Copy)]
struct ValveSnapshot {
    is_open: bool,
    default_open_duration: Option<u32>,
    open_duration: Option<u32>,
    remaining_duration: Option<u32>,
}

impl ValveSnapshot {
    fn new(valve: &Valve, now: u64) -> Self {
        Self {
            is_open: valve.is_open,
            default_open_duration: Some(valve.default_open_duration),
            open_duration: valve.open_duration,
            remaining_duration: if valve.is_open && valve.open_duration.is_some() {
                remaining_duration_seconds(valve.expiration_ticks, now)
            } else {
                None
            },
        }
    }

    fn state(self) -> ValveStateEnum {
        if self.is_open {
            ValveStateEnum::Open
        } else {
            ValveStateEnum::Closed
        }
    }
}

fn remaining_duration_seconds(expiration_ticks: Option<u64>, now: u64) -> Option<u32> {
    expiration_ticks.map(|expiration| {
        let remaining_us = expiration.saturating_sub(now);
        let remaining_seconds = remaining_us.div_ceil(MICROSECONDS_PER_SECOND);
        remaining_seconds.min(u64::from(u32::MAX - 1)) as u32
    })
}

fn read_default_open_duration(device: LibertasDevice) -> Option<ValveData> {
    libertas_data_read(
        DEFAULT_OPEN_DURATION_RESOURCE,
        &[NotificationArgument::Object(device)],
    )
}

fn persist_default_open_duration(device: LibertasDevice, value: u32) {
    libertas_data_write(
        DEFAULT_OPEN_DURATION_RESOURCE,
        &[NotificationArgument::Object(device)],
        &ValveData::DefaultOpenDuration { value },
    );
}

fn load_default_open_duration(device: LibertasDevice) -> u32 {
    match read_default_open_duration(device) {
        Some(ValveData::DefaultOpenDuration { value }) if value != 0 => value,
        _ => {
            let value = DEFAULT_OPEN_DURATION_SECONDS;
            persist_default_open_duration(device, value);
            value
        }
    }
}

fn report_dynamic_attributes(device: LibertasDevice) {
    let mut changed = MatterReadCluster::<4, 0>::new(clusters::ValveConfigurationandControl);
    if changed.add_attribute::<OpenDuration>().is_err()
        || changed.add_attribute::<RemainingDuration>().is_err()
        || changed.add_attribute::<CurrentState>().is_err()
        || changed.add_attribute::<TargetState>().is_err()
    {
        return;
    }
    if let Ok(request) = changed.request() {
        let _ = MatterDevice::new(device)
            .changed_batch(LIBERTAS_BROADCAST_DEST, core::slice::from_ref(&request));
    }
}

fn report_default_open_duration(device: LibertasDevice) {
    let mut changed = MatterReadCluster::<1, 0>::new(clusters::ValveConfigurationandControl);
    if changed.add_attribute::<DefaultOpenDuration>().is_err() {
        return;
    }
    if let Ok(request) = changed.request() {
        let _ = MatterDevice::new(device)
            .changed_batch(LIBERTAS_BROADCAST_DEST, core::slice::from_ref(&request));
    }
}

fn report_all_attributes(device: LibertasDevice) {
    let mut changed = MatterReadCluster::<5, 0>::new(clusters::ValveConfigurationandControl);
    if changed.add_attribute::<OpenDuration>().is_err()
        || changed.add_attribute::<DefaultOpenDuration>().is_err()
        || changed.add_attribute::<RemainingDuration>().is_err()
        || changed.add_attribute::<CurrentState>().is_err()
        || changed.add_attribute::<TargetState>().is_err()
    {
        return;
    }
    if let Ok(request) = changed.request() {
        let _ = MatterDevice::new(device)
            .changed_batch(LIBERTAS_BROADCAST_DEST, core::slice::from_ref(&request));
    }
}

fn close_valve(valve: &Rc<RefCell<Valve>>) {
    let device = {
        let mut valve = valve.borrow_mut();
        if !valve.is_open && valve.open_duration.is_none() && valve.expiration_ticks.is_none() {
            return;
        }

        valve.is_open = false;
        valve.open_duration = None;
        valve.expiration_ticks = None;
        if let Some(timer) = valve.timer {
            libertas_timer_cancel(timer);
        }
        valve.device
    };

    report_dynamic_attributes(device);
}

fn open_valve(
    target_valve: &Rc<RefCell<Valve>>,
    shared: &Rc<RefCell<IrrigationContext>>,
    duration_seconds: Option<u32>,
) {
    let valves_to_close: Vec<_> = {
        let shared = shared.borrow();
        shared
            .valves
            .iter()
            .filter(|valve| !Rc::ptr_eq(valve, target_valve))
            .cloned()
            .collect()
    };
    for valve in valves_to_close {
        close_valve(&valve);
    }

    let device = {
        let mut valve = target_valve.borrow_mut();
        valve.is_open = true;
        valve.open_duration = duration_seconds;

        match duration_seconds {
            Some(duration) => {
                let expiration = libertas_get_sys_ticks()
                    .saturating_add(u64::from(duration) * MICROSECONDS_PER_SECOND);
                valve.expiration_ticks = Some(expiration);

                if let Some(timer) = valve.timer {
                    libertas_timer_update_interval(timer, expiration);
                } else {
                    let timer_context = Box::new(TimerContext {
                        valve: Rc::clone(target_valve),
                    });
                    valve.timer = Some(libertas_timer_new_interval(
                        expiration,
                        |_timer_id, _current_ticks, context| {
                            let context = context.downcast_mut::<TimerContext>().unwrap();
                            close_valve(&context.valve);
                        },
                        timer_context,
                    ));
                }
            }
            None => {
                valve.expiration_ticks = None;
                if let Some(timer) = valve.timer {
                    libertas_timer_cancel(timer);
                }
            }
        }

        valve.device
    };

    report_dynamic_attributes(device);
}

fn write_attribute_path<W: TLVWrite + ?Sized>(
    writer: &mut W,
    tag: Tag,
    cluster_id: u32,
    attribute_id: u32,
) -> Result<(), Error> {
    writer.start_list(tag)?;
    writer.u32(Tag::Context(3), cluster_id)?;
    writer.u32(Tag::Context(4), attribute_id)?;
    writer.end_container()
}

fn write_attribute_data<W, A>(writer: &mut W, value: &A) -> Result<(), Error>
where
    W: TLVWrite + ?Sized,
    A: MatterAttribute,
{
    writer.start_struct(Tag::Anonymous)?;
    writer.start_struct(Tag::Context(1))?;
    write_attribute_path(writer, Tag::Context(1), A::CLUSTER_ID, A::ID)?;
    value.to_tlv(Tag::Context(2), writer)?;
    writer.end_container()?;
    writer.end_container()
}

fn write_attribute_status<W: TLVWrite + ?Sized>(
    writer: &mut W,
    cluster_id: u32,
    attribute_id: u32,
    status: IMStatusCode,
) -> Result<(), Error> {
    writer.start_struct(Tag::Anonymous)?;
    writer.start_struct(Tag::Context(0))?;
    write_attribute_path(writer, Tag::Context(0), cluster_id, attribute_id)?;
    writer.start_struct(Tag::Context(1))?;
    writer.u8(Tag::Context(0), status as u8)?;
    writer.end_container()?;
    writer.end_container()?;
    writer.end_container()
}

fn send_matter_response(
    context: MatterRequestContext,
    operation: Operation,
    buffer: &InlineByteBuffer,
) {
    libertas_device_send_response(
        PROTOCOL_MATTER,
        context.device.id(),
        operation as u8,
        buffer.as_slice(),
        context.transaction_id,
        context.peer,
    );
}

fn handle_read_request(
    context: MatterRequestContext,
    data: &[u8],
    valve: &Rc<RefCell<Valve>>,
) -> Result<(), Error> {
    let root = Element::from_bytes(data)?;
    if root.value_type() != ValueType::Array {
        return Err(Error::TypeMismatch);
    }

    let now = libertas_get_sys_ticks();
    let snapshot = ValveSnapshot::new(&valve.borrow(), now);
    let mut response = InlineByteBuffer::new();
    response.start_array(Tag::Anonymous)?;

    let mut paths = root.children()?;
    while let Some(path) = paths.read_element()? {
        if path.value_type() != ValueType::List {
            return Err(Error::TypeMismatch);
        }
        let cluster_id = path.context(3)?.u32()?;
        let attribute_id = path.context(4)?.u32()?;

        if cluster_id != clusters::ValveConfigurationandControl {
            write_attribute_status(
                &mut response,
                cluster_id,
                attribute_id,
                IMStatusCode::UnsupportedCluster,
            )?;
            continue;
        }

        match attribute_id {
            valve_attributes::OpenDuration => write_attribute_data(
                &mut response,
                &OpenDuration(Nullable::from(snapshot.open_duration)),
            )?,
            valve_attributes::DefaultOpenDuration => write_attribute_data(
                &mut response,
                &DefaultOpenDuration(Nullable::from(snapshot.default_open_duration)),
            )?,
            valve_attributes::RemainingDuration => write_attribute_data(
                &mut response,
                &RemainingDuration(Nullable::from(snapshot.remaining_duration)),
            )?,
            valve_attributes::CurrentState => write_attribute_data(
                &mut response,
                &CurrentState(Nullable::some(snapshot.state())),
            )?,
            valve_attributes::TargetState => {
                write_attribute_data(&mut response, &TargetState(Nullable::null()))?
            }
            _ => write_attribute_status(
                &mut response,
                cluster_id,
                attribute_id,
                IMStatusCode::UnsupportedAttribute,
            )?,
        }
    }

    response.end_container()?;
    send_matter_response(context, Operation::ReportData, &response);
    Ok(())
}

fn status_for_decode_error(error: Error) -> IMStatusCode {
    match error {
        Error::Constraint | Error::OutOfRange => IMStatusCode::ConstraintError,
        Error::TypeMismatch
        | Error::Malformed
        | Error::Truncated
        | Error::MissingField(_)
        | Error::InvalidUtf8 => IMStatusCode::InvalidDataType,
        _ => IMStatusCode::InvalidAction,
    }
}

fn validate_default_open_duration(value: Nullable<u32>) -> Result<u32, IMStatusCode> {
    match value.into_option() {
        Some(value) if value != 0 => Ok(value),
        _ => Err(IMStatusCode::ConstraintError),
    }
}

fn handle_write_request(
    context: MatterRequestContext,
    data: &[u8],
    valve: &Rc<RefCell<Valve>>,
) -> Result<(), Error> {
    let root = Element::from_bytes(data)?;
    if root.value_type() != ValueType::Array {
        return Err(Error::TypeMismatch);
    }

    let mut response = InlineByteBuffer::new();
    response.start_array(Tag::Anonymous)?;
    let mut default_changed = false;

    let mut entries = root.children()?;
    while let Some(entry) = entries.read_element()? {
        if entry.value_type() != ValueType::Structure {
            return Err(Error::TypeMismatch);
        }
        let path = entry.context(1)?;
        if path.value_type() != ValueType::List {
            return Err(Error::TypeMismatch);
        }
        let cluster_id = path.context(3)?.u32()?;
        let attribute_id = path.context(4)?.u32()?;

        let status = if cluster_id != clusters::ValveConfigurationandControl {
            IMStatusCode::UnsupportedCluster
        } else if attribute_id == valve_attributes::DefaultOpenDuration {
            match entry
                .context(2)
                .and_then(|value| DefaultOpenDuration::from_tlv(&value))
            {
                Ok(DefaultOpenDuration(value)) => match validate_default_open_duration(value) {
                    Ok(new_value) => {
                        let mut valve = valve.borrow_mut();
                        default_changed |= valve.default_open_duration != new_value;
                        valve.default_open_duration = new_value;
                        IMStatusCode::Success
                    }
                    Err(status) => status,
                },
                Err(error) => status_for_decode_error(error),
            }
        } else if matches!(
            attribute_id,
            valve_attributes::OpenDuration
                | valve_attributes::RemainingDuration
                | valve_attributes::CurrentState
                | valve_attributes::TargetState
        ) {
            IMStatusCode::UnsupportedWrite
        } else {
            IMStatusCode::UnsupportedAttribute
        };

        write_attribute_status(&mut response, cluster_id, attribute_id, status)?;
    }

    response.end_container()?;
    send_matter_response(context, Operation::WriteResponse, &response);

    if default_changed {
        let valve = valve.borrow();
        let device = valve.device;
        persist_default_open_duration(device, valve.default_open_duration);
        report_default_open_duration(device);
    }
    Ok(())
}

fn resolve_open_duration(
    requested: Option<Nullable<u32>>,
    default_duration: Option<u32>,
) -> Result<Option<u32>, IMStatusCode> {
    let duration = match requested {
        Some(Nullable::Null) => None,
        Some(Nullable::Value(duration)) => Some(duration),
        None => default_duration,
    };

    if duration == Some(0) {
        Err(IMStatusCode::ConstraintError)
    } else {
        Ok(duration)
    }
}

fn send_command_status(
    context: MatterRequestContext,
    cluster_id: u32,
    command_id: u32,
    status: IMStatusCode,
) -> Result<(), Error> {
    let mut response = InlineByteBuffer::new();
    frame::encode_command_status(&mut response, cluster_id, command_id, Status::from(status))?;
    send_matter_response(context, Operation::InvokeResponse, &response);
    Ok(())
}

fn handle_invoke_request(
    context: MatterRequestContext,
    data: &[u8],
    valve: &Rc<RefCell<Valve>>,
    shared: &Rc<RefCell<IrrigationContext>>,
) -> Result<(), Error> {
    let command = frame::decode_command_request(data)?;
    let status = if command.cluster_id != clusters::ValveConfigurationandControl {
        IMStatusCode::UnsupportedCluster
    } else {
        match command.command_id {
            valve_commands::Open => match decode_command::<Open>(data) {
                Ok(command) if command.TargetLevel.is_some() => IMStatusCode::InvalidCommand,
                Ok(command) => {
                    let default_duration = Some(valve.borrow().default_open_duration);
                    match resolve_open_duration(command.OpenDuration, default_duration) {
                        Ok(duration) => {
                            open_valve(valve, shared, duration);
                            IMStatusCode::Success
                        }
                        Err(status) => status,
                    }
                }
                Err(error) => status_for_decode_error(error),
            },
            valve_commands::Close => match decode_command::<Close>(data) {
                Ok(_) => {
                    close_valve(valve);
                    IMStatusCode::Success
                }
                Err(error) => status_for_decode_error(error),
            },
            _ => IMStatusCode::UnsupportedCommand,
        }
    };

    send_command_status(context, command.cluster_id, command.command_id, status)
}

fn status_for_handler_error(error: Error) -> IMStatusCode {
    match error {
        Error::NoSpace => IMStatusCode::ResourceExhausted,
        Error::Constraint | Error::OutOfRange => IMStatusCode::ConstraintError,
        Error::PathMismatch => IMStatusCode::NotFound,
        Error::UnsupportedAccess => IMStatusCode::UnsupportedAccess,
        Error::TypeMismatch
        | Error::Malformed
        | Error::Truncated
        | Error::MissingField(_)
        | Error::InvalidUtf8 => IMStatusCode::InvalidAction,
        _ => IMStatusCode::Failure,
    }
}

/// Emulate a multi-zone sprinkler.
///
/// Each valve is a binary Matter Valve Configuration and Control server. The
/// controller permits only one valve to be open at a time.
#[libertas_data_schema(ValveData)]
#[libertas_string_resources(APP_STRINGS)]
pub fn virtual_irrigation_controller(
    /*
     * A list of sprinkler valves.
     * #[libertas_size(min=1)]
     * ----
     * A sprinkler valve.
     * #[libertas_virtual_device_type("BQEBQAEBgQEBAQAABQABAwQFAAIAAQA=")]
     */
    valves: Vec<LibertasVirtualDevice>,
) {
    let shared = Rc::new(RefCell::new(IrrigationContext {
        valves: Vec::with_capacity(valves.len()),
    }));

    for &valve_device in &valves {
        let valve = Rc::new(RefCell::new(Valve {
            device: valve_device,
            is_open: false,
            default_open_duration: load_default_open_duration(valve_device),
            open_duration: None,
            expiration_ticks: None,
            timer: None,
        }));
        shared.borrow_mut().valves.push(Rc::clone(&valve));

        let context = Box::new(ValveContext {
            valve: Rc::clone(&valve),
            shared: Rc::clone(&shared),
        });

        libertas_register_device_listener(
            valve_device,
            move |device, opcode, data, context, transaction_id, peer| {
                let context = context.downcast_mut::<ValveContext>().unwrap();
                let request = MatterRequestContext::new(device, transaction_id, peer);
                let result = match opcode {
                    opcode
                        if opcode == Operation::ReadRequest as u8
                            || opcode == Operation::SubscribeRequest as u8 =>
                    {
                        handle_read_request(request, data, &context.valve)
                    }
                    opcode if opcode == Operation::WriteRequest as u8 => {
                        handle_write_request(request, data, &context.valve)
                    }
                    opcode if opcode == Operation::InvokeRequest as u8 => {
                        handle_invoke_request(request, data, &context.valve, &context.shared)
                    }
                    _ => {
                        request.respond_status(IMStatusCode::InvalidAction);
                        Ok(())
                    }
                };

                if let Err(error) = result {
                    request.respond_status(status_for_handler_error(error));
                }
            },
            context,
        );

        report_all_attributes(valve_device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valve_data_round_trips_through_avro() {
        let data = ValveData::DefaultOpenDuration { value: 90 };

        assert_eq!(ValveData::from_avro(&data.to_avro()), Ok(data));
    }

    #[test]
    fn default_open_duration_is_required_and_nonzero() {
        assert_eq!(
            validate_default_open_duration(Nullable::Null),
            Err(IMStatusCode::ConstraintError)
        );
        assert_eq!(
            validate_default_open_duration(Nullable::some(0)),
            Err(IMStatusCode::ConstraintError)
        );
        assert_eq!(validate_default_open_duration(Nullable::some(600)), Ok(600));
    }

    #[test]
    fn command_duration_overrides_default() {
        assert_eq!(
            resolve_open_duration(
                Some(Nullable::some(45)),
                Some(DEFAULT_OPEN_DURATION_SECONDS)
            ),
            Ok(Some(45))
        );
    }

    #[test]
    fn explicit_null_requests_indefinite_open() {
        assert_eq!(
            resolve_open_duration(Nullable::Null.into(), Some(DEFAULT_OPEN_DURATION_SECONDS)),
            Ok(None)
        );
    }

    #[test]
    fn omitted_duration_uses_writable_default() {
        assert_eq!(resolve_open_duration(None, Some(90)), Ok(Some(90)));
        assert_eq!(resolve_open_duration(None, None), Ok(None));
    }

    #[test]
    fn zero_duration_is_rejected() {
        assert_eq!(
            resolve_open_duration(Some(Nullable::some(0)), Some(90)),
            Err(IMStatusCode::ConstraintError)
        );
    }

    #[test]
    fn remaining_duration_rounds_up_until_expiration() {
        let expiration = Some(5 * MICROSECONDS_PER_SECOND);
        assert_eq!(remaining_duration_seconds(expiration, 0), Some(5));
        assert_eq!(
            remaining_duration_seconds(expiration, MICROSECONDS_PER_SECOND + 1),
            Some(4)
        );
        assert_eq!(
            remaining_duration_seconds(expiration, 5 * MICROSECONDS_PER_SECOND),
            Some(0)
        );
    }
}
