//! A Matter multi-zone sprinkler controller emulator.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
use alloc::rc::Rc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::cell::RefCell;

use libertas::*;
use libertas_matter::*;
use libertas_matter::utils::storage::WriteBuf;
use libertas_matter::tlv::*;
use libertas_matter::im::IMStatusCode;
use libertas_matter::error::Error;

const CLUSTER_ON_OFF: u32 = 0x0006;
const ATTR_ON_OFF: u32 = 0x0000;
const ATTR_ON_TIME: u32 = 16385; // 0x4001

const CMD_OFF: u32 = 0;
const CMD_ON: u32 = 1;
const CMD_ON_WITH_TIMED_OFF: u32 = 66;

const DEFAULT_TIMEOUT_MS: u32 = 10 * 60 * 1000;

struct Valve {
    device: LibertasDevice,
    is_on: bool,
    timer: Option<u32>,
    expire_ticks: u64,
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
    _shared: Rc<RefCell<IrrigationContext>>,
}

fn report_attributes_changed(device: LibertasDevice) {
    let mut req = LibertasClusterReadReq::new(CLUSTER_ON_OFF);
    req.attributes.push(ATTR_ON_OFF);
    req.attributes.push(ATTR_ON_TIME);
    libertas_virtual_device_attribute_changed(device, &[req], LIBERTAS_BROADCAST_DEST);
    libertas_log(LogLevel::Debug, "report_attributes_changed");
}

fn turn_off_valve(valve: &Rc<RefCell<Valve>>) {
    let mut v = valve.borrow_mut();
    if v.is_on {
        v.is_on = false;
        if let Some(timer) = v.timer {
            libertas_timer_cancel(timer);
        }
        report_attributes_changed(v.device);
    }
}

fn turn_on_valve(
    target_valve: &Rc<RefCell<Valve>>,
    shared: &Rc<RefCell<IrrigationContext>>,
    on_time_ms: u32,
) {
    // Turn off all other valves
    let valves = &shared.borrow().valves;
    let target_device = target_valve.borrow().device;
    for valve in valves {
        if valve.borrow().device != target_device {
            turn_off_valve(valve);
        }
    }

    let mut v = target_valve.borrow_mut();
    v.is_on = true;
    let now = libertas_get_sys_ticks();
    let duration_ticks = (on_time_ms as u64) * 1000;
    let expire = now + duration_ticks;
    v.expire_ticks = expire;

    if let Some(timer) = v.timer {
        libertas_timer_update_interval(timer, expire);
    } else {
        let timer_context = Box::new(TimerContext {
            valve: Rc::clone(target_valve),
            _shared: Rc::clone(shared),
        });
        let new_timer = libertas_timer_new_interval(
            expire,
            |_timer_id, _cur_ticks, tag_any| {
                let context = tag_any.downcast_mut::<TimerContext>().unwrap();
                turn_off_valve(&context.valve);
            },
            timer_context,
        );
        v.timer = Some(new_timer);
    }

    report_attributes_changed(v.device);
}

fn handle_read_request(
    device: LibertasDevice,
    trans_id: u32,
    data: &[u8],
    valve: &Rc<RefCell<Valve>>,
    peer: u32,
) -> Result<(), Error> {
    let mut buf = LibertasUninitStackbuf::new();
    let mut wb = WriteBuf::new(buf.as_mut_slice());

    libertas_virtual_device_attributes_rsp_prepare(&mut wb)?;

    let array = TLVElement::new(data).array()?;
    for path_elem in array.iter() {
        let path = path_elem?.list()?;
        let cluster_id = path.ctx(3)?.u32()?;
        let attribute_id = path.ctx(4)?.u32()?;

        if cluster_id == CLUSTER_ON_OFF {
            if attribute_id == ATTR_ON_OFF {
                libertas_virtual_device_attributes_rsp_add_prepare(&mut wb, cluster_id, attribute_id)?;
                wb.bool(&TLVTag::Context(2), valve.borrow().is_on)?;
                libertas_virtual_device_attributes_rsp_add_finalize(&mut wb)?;
            } else if attribute_id == ATTR_ON_TIME {
                libertas_virtual_device_attributes_rsp_add_prepare(&mut wb, cluster_id, attribute_id)?;
                let mut remaining_tenths = 0;
                let v = valve.borrow();
                if v.is_on && v.timer.is_some() {
                    let now = libertas_get_sys_ticks();
                    if v.expire_ticks > now {
                        let remaining_us = v.expire_ticks - now;
                        remaining_tenths = ((remaining_us + 50_000) / 100_000) as u16;
                    }
                }
                wb.u16(&TLVTag::Context(2), remaining_tenths)?;
                libertas_virtual_device_attributes_rsp_add_finalize(&mut wb)?;
            } else {
                libertas_virtual_device_attributes_rsp_add_status(&mut wb, cluster_id, attribute_id, IMStatusCode::UnsupportedAttribute)?;
            }
        } else {
            libertas_virtual_device_attributes_rsp_add_status(&mut wb, cluster_id, attribute_id, IMStatusCode::UnsupportedCluster)?;
        }
    }

    libertas_virtual_device_attributes_rsp_finalize(&mut wb)?;
    libertas_virtual_device_attributes_rsp_send(device, trans_id, wb.as_slice(), peer);
    Ok(())
}

fn handle_invoke_request(
    device: LibertasDevice,
    trans_id: u32,
    data: &[u8],
    valve: &Rc<RefCell<Valve>>,
    shared: &Rc<RefCell<IrrigationContext>>,
    peer: u32,
) -> Result<(), Error> {
    let (cluster_id, command_id, fields) = libertas_virtual_device_invoke_req_parse(data)?;

    let mut status = IMStatusCode::Success;

    if cluster_id == CLUSTER_ON_OFF {
        if command_id == CMD_ON {
            turn_on_valve(valve, shared, DEFAULT_TIMEOUT_MS);
        } else if command_id == CMD_OFF {
            turn_off_valve(valve);
        } else if command_id == CMD_ON_WITH_TIMED_OFF {
            let fields_struct = fields.structure()?;
            let on_time_tenths = fields_struct.ctx(1)?.u16()?;
            let on_time_ms = (on_time_tenths as u32) * 100;
            turn_on_valve(valve, shared, on_time_ms);
        } else {
            status = IMStatusCode::UnsupportedCommand;
        }
    } else {
        status = IMStatusCode::UnsupportedCluster;
    }

    let mut buf = LibertasUninitStackbuf::new();
    let mut wb = WriteBuf::new(buf.as_mut_slice());
    libertas_virtual_device_invoke_rsp_status(&mut wb, cluster_id, command_id, status as u32)?;
    libertas_virtual_device_invoke_rsp_send(device, trans_id, wb.as_slice(), peer);
    Ok(())
}

/// Emulate a multi-zone sprinkler.
/// 
pub fn virtual_irrigation_controller(
    /*
     * A list of sprinkler valves.
     * #[libertas_array(sizeMin=1)]
     * ----
     * A sprinkler valve.
     * #[libertas_virtual_device_type("BQEBQAEBBgEBAAACAIGAAQADAAFCAA==")]
     */
    valves: Vec<LibertasVirtualDevice>) {
    let shared = Rc::new(RefCell::new(IrrigationContext {
        valves: Vec::with_capacity(valves.len()),
    }));

    for &valve_device in &valves {
        let valve = Rc::new(RefCell::new(Valve {
            device: valve_device,
            is_on: false,
            timer: None,
            expire_ticks: 0,
        }));
        shared.borrow_mut().valves.push(Rc::clone(&valve));

        let context = Box::new(ValveContext {
            valve: Rc::clone(&valve),
            shared: Rc::clone(&shared),
        });

        libertas_register_device_listener(
            valve_device,
            move |device, opcode, data, tag_any, trans_id, peer| {
                let ctx = tag_any.downcast_mut::<ValveContext>().unwrap();
                let trans_id_val = trans_id.unwrap_or(0);
                match opcode {
                    o if o == OpCode::ReadRequest as u8 || o == OpCode::SubscribeRequest as u8 => {
                        let _ = handle_read_request(device, trans_id_val, data, &ctx.valve, peer);
                    }
                    o if o == OpCode::InvokeRequest as u8 => {
                        let _ = handle_invoke_request(device, trans_id_val, data, &ctx.valve, &ctx.shared, peer);
                    }
                    _ => {
                        libertas_virtual_device_status_rsp_send(device, trans_id_val, IMStatusCode::InvalidAction, peer);
                    }
                }
            },
            context,
        );

        report_attributes_changed(valve_device);
    }
}
