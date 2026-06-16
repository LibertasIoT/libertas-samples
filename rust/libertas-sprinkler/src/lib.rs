//! Libertas Sprinkler Agent
//! This is a sample to demonstrate Libertas Rust SDK.
#![forbid(unsafe_code)]

extern crate alloc;
use alloc::vec::Vec;
use alloc::rc::Rc;
use core::cell::RefCell;
use libertas::*;
use libertas_macros::*;
//use libertas_matter::*;

pub static APP_STRINGS: [(&str, &str); 2] = [
    ("HOLD_OFF_UPDATED", "The hold-off list for %1$s has been updated."),
    ("A_RESOURCE_NAME", "A printf style template"),
];

/// A time period
/// Including a start time and a duration.
#[derive(Clone, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct TimeSlot {
    /// Start time
    /// The start time
    pub start_time: LibertasDateTime,
    /// Duration
    /// The duration in seconds.
    #[libertas_time_interval]
    pub duration: u32,
}

/// ZoneDataProtocol
/// The runtime protocol of the sprinkler agent
#[derive(Clone, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum ZoneDataProtocol {
    /// Get zone information
    /// Get the sprinkler runtime information for the zone
    #[libertas_request]
    #[libertas_subscription_request]
    #[libertas_cacheable]
    #[libertas_next_response(ZoneInfo)]
    GetZoneInfo,
    /// Zone information
    /// Information about the sprinkler zone's next watering schedule and hold-off period list. A hold-off ensures the zone won't be watered during the period.
    #[libertas_response]
    #[libertas_next_request(UpdateHoldOff)]
    ZoneInfo {
        /// Next watering schedule
        /// The next scheduled watering time and duration. This schedule is dynamically calculated and may change later. The calculation must avoid the hold-off periods.
        next_schedule: TimeSlot,
        /// Hold off periods
        /// The list of sprinkler's hold-off period that the dynamic calculation of watering schedule must avoid. A hold-off ensures a sprinkler zone (area) won't be watered during the period.
        /// ----
        /// Hold off period
        /// A start time and duration of a watering hold-off.
        hold_off_periods: Vec<TimeSlot>,
    },
    /// Update hold off periods
    /// A request to update the entire list of "hold off" periods of the sprinkler zone. The system ensures the zone won't be watered during each hold off period.
    #[libertas_request]
    #[libertas_next_response(ZoneInfo)]
    UpdateHoldOff {
        /// Hold off periods
        /// The list of sprinkler's hold-off period that the dynamic calculation of watering schedule must avoid. A hold-off ensures a sprinkler zone (area) won't be watered during the period.
        /// ----
        /// Hold off period
        /// A start time and duration of a watering hold-off.
        #[libertas_copy_from("$.hold_off_periods")]
        hold_off_periods: Vec<TimeSlot>,
    },
}

#[repr(u8)]
#[derive(Clone, LibertasAvroDecode)]
pub enum SoilType { Loam, Clay, ClayLoam, 
    SiltyClay, SandyLoam, LoamySand, Sand }


#[repr(u8)]
#[derive(Clone, LibertasAvroDecode)]
pub enum PlantType {Lawn, FruitTrees, Flowers,
    Vegetables, Citrus, TreesBushes, Xeriscape }


#[repr(u8)]
#[derive(Clone, LibertasAvroDecode)]
pub enum SprinklerHead {SurfaceDrip, Bubblers,
    PopupSpray, RotorsLowRate, RotorsHighRate }

/// Sprinkler zone config    
/// Configuration of a zone
#[derive(Clone, LibertasAvroDecode, LibertasExport)]
pub struct SprinklerZone {
    /// Zone
    /// In an irrigation system, a sprinkler zone is a specific group of sprinklers that are controlled by a single dedicated valve and operate simultaneously to water an area of plants.
    #[libertas_device_type("BQEBAUABBgI=")]
    #[libertas_ui_header]
    #[libertas_unique]
    pub zone_valve: LibertasDevice,
    /// Field_capacity
    #[libertas_number(max=100)]
    pub field_capacity: u8,
    /// Soil type
    #[libertas_default(Loam)]
    pub soil_type: SoilType,
    pub plant_type: PlantType,
    pub head: SprinklerHead,
    #[libertas_endpoint_schema(ZoneDataProtocol)]
    #[libertas_endpoint_server]
    #[libertas_endpoint_base_objects("^.zone_valve")]
    pub zone_info: LibertasEndpoint,
}

/// Libertas sprinkler agent
/// The user configures the information about each zone.
/// It automatically calculate the optimal watering schedule for each zone based on onfomration such as
/// watering history, weather forecast, etc.
/// During runtime, the user can maintain a list of "hold-off" periods on each zone when watering shall be prevented.
/// The hold-off period list will affect the watring schedule acting as a constraint.
#[libertas_string_resources(APP_STRINGS)]
pub fn libertas_sprinkler (
    /*
     * Notification list
     * A list of users and groups that receive notifications when hold-off periods are updated.
     * ----
     * A user or group
     * #[libertas_unique]
     */
    notification_list: Vec<LibertasUser>,
    /*
     * Zones
     * List of sprinkler zones (watering areas).
     * #[libertas_array(sizeMin=1)]
     * ----
     * Zone config
     * Configuration of one sprinkler zone.
     */
    zones: Vec<SprinklerZone>) {
    let mut cur_start_time: u64 = libertas_get_utc_time().unwrap() / 1000000;     // us to seconds
    cur_start_time /= 60;           // round down to the nearest minute so that it's easier to read out
    cur_start_time *= 60;
    cur_start_time += 24 * 3600;    // start from next day
    let cur_duration = 1000;   // seconds
    for zone in zones {
        let tag = Rc::new(
            RefCell::new(
                ZoneData{
                    zone: zone.clone(),
                    next_schedule: TimeSlot {
                        start_time: cur_start_time,     // us to seconds
                        duration: cur_duration,
                    },
                    hold_off_periods: Vec::new(),
                    notification_list: notification_list.clone(),
                }
            ));
        cur_start_time = cur_start_time + cur_duration as u64;
        libertas_register_endpoint_listener(zone.zone_info, |device, opcode, protocol: Option<ZoneDataProtocol>, context, trans_id, peer| {
            let mut data = context.downcast_mut::<Rc<RefCell<ZoneData>>>().unwrap().borrow_mut();
            if let Some(protocol) = protocol {
                if let Some(trans_id) = trans_id {
                    match protocol {
                        ZoneDataProtocol::GetZoneInfo => {
                            if opcode == OP_ENDPOINT_REQ {
                                send_data(&*data, Some(trans_id), peer);
                            } else if opcode == OP_ENDPOINT_SUB_REQ {
                                let rsp = ZoneDataProtocol::GetZoneInfo;
                                libertas_endpoint_response(device, &rsp, trans_id, peer);
                                send_data(&*data, None, peer);
                            }
                        },
                        ZoneDataProtocol::UpdateHoldOff { hold_off_periods } => {
                            data.hold_off_periods = hold_off_periods;
                            // sort hold off periods by start time
                            data.hold_off_periods.sort_by_key(|h| h.start_time);
                            // If there is an overlay between the next schedule and any hold off period, we shift 
                            // the next schedule to after the hold off period.
                            // 2. Logic to shift the next schedule if it overlaps with any hold-off period
                            // We loop because shifting past one hold-off might put us into the middle of the next one
                            let mut changed = true;
                            while changed {
                                changed = false;
                                let schedule_start = data.next_schedule.start_time;
                                let schedule_end = schedule_start + data.next_schedule.duration as u64;

                                for hold_off in &data.hold_off_periods {
                                    let hold_off_start = hold_off.start_time;
                                    let hold_off_end = hold_off_start + hold_off.duration as u64;

                                    // Check for overlap: (StartA < EndB) and (EndA > StartB)
                                    if schedule_start < hold_off_end && schedule_end > hold_off_start {
                                        // Shift schedule to immediately after this hold-off period
                                        data.next_schedule.start_time = hold_off_end;
                                        changed = true;
                                        // Once we shift, we must re-check against all periods 
                                        // (especially subsequent ones)
                                        break; 
                                    }
                                }
                            }

                            let arguments: [NotificationArgument; 1] = [
                                NotificationArgument::Object(data.zone.zone_valve),
                            ];
                            libertas_notification_send(
                                &data.notification_list, 
                                NotificationImportance::AlertLow,
                                Some(data.zone.zone_valve), 
                                "HOLD_OFF_UPDATED", 
                                &arguments);
                            let d = &*data;
                            send_data(d, Some(trans_id), peer);
                            send_data(d, None, LIBERTAS_BROADCAST_DEST);
                        },
                        _ => {},
                    }
                }
            }
        }, Box::new(Rc::clone(&tag)));
    }
}

struct ZoneData {
    zone: SprinklerZone,
    next_schedule: TimeSlot,
    hold_off_periods: Vec<TimeSlot>,
    notification_list: Vec<LibertasUser>
}

fn send_data(zone_data: &ZoneData, trans_id: Option<LibertasTransId>, peer: u32) {
    let info = ZoneDataProtocol::ZoneInfo{
        next_schedule: zone_data.next_schedule.clone(),
        hold_off_periods: zone_data.hold_off_periods.clone(),
    };
    if let Some(trans_id) = trans_id {
        libertas_endpoint_response(zone_data.zone.zone_info, &info, trans_id, peer);
    } else {
        libertas_endpoint_report(zone_data.zone.zone_info, &info, Some(peer));
    }
}