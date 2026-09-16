//! Durable timed-run accounting and asynchronous shutdown.

use super::*;
use libertas::{libertas_register_shutdown_handler, libertas_shutdown_complete};

const YIELD_MICROSECONDS: u64 = 5_000;

pub(super) fn valid(checkpoint: &SprinklerRestartAccountingV1) -> bool {
    let SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        duration_seconds,
        watering_percent,
        applied_water_millimeters,
    } = checkpoint.segment
    else {
        return false;
    };
    valid_watering_activity(&checkpoint.activity)
        && checkpoint.activity.actual_starts_at.is_some_and(|start| {
            start <= checkpoint.accounted_through
                && checkpoint.expected_ends_at.is_none_or(|end| end >= start)
        })
        && matches!(
            checkpoint.activity.outcome,
            SprinklerWateringOutcomeV1::Running
                | SprinklerWateringOutcomeV1::Completed
                | SprinklerWateringOutcomeV1::Failed
        )
        && starts_at.checked_add(u64::from(duration_seconds)) == Some(checkpoint.accounted_through)
        && water_event_index(&checkpoint.segment).is_some()
        && valid_watering_percent(watering_percent)
        && valid_nonnegative(applied_water_millimeters)
        && checkpoint.delivery_millimeters_per_hour.is_finite()
        && checkpoint.delivery_millimeters_per_hour > 0.0
}

pub(super) fn capture(zone: &ZoneRuntime) -> Option<SprinklerRestartAccountingV1> {
    let activity = zone.current_activity.as_ref()?;
    let actual_start = activity.actual_starts_at?;
    from_activity(
        activity,
        &zone.water_events,
        zone.accounted_at_utc.unwrap_or(actual_start),
        zone.memory.watering_percent,
        zone.configuration.sprinkler_head_type,
    )
}

fn from_activity(
    activity: &SprinklerWateringActivityV1,
    events: &[SprinklerWaterEventV1],
    accounted_through: u64,
    watering_percent: u16,
    head: SprinklerHeadTypeV1,
) -> Option<SprinklerRestartAccountingV1> {
    let actual_start = activity.actual_starts_at?;
    if activity.outcome != SprinklerWateringOutcomeV1::Running {
        return None;
    }
    let segment = events
        .iter()
        .rev()
        .find(|event| {
            matches!(event, SprinklerWaterEventV1::IrrigationV1 { watering_percent: percent, .. }
                if *percent == watering_percent)
                && event.ends_at() == Some(accounted_through)
        })
        .cloned()
        .unwrap_or(SprinklerWaterEventV1::IrrigationV1 {
            starts_at: accounted_through,
            duration_seconds: 0,
            watering_percent,
            applied_water_millimeters: 0.0,
        });
    let checkpoint = SprinklerRestartAccountingV1 {
        activity: activity.clone(),
        accounted_through,
        segment,
        expected_ends_at: (activity.origin == SprinklerWateringOriginV1::Automatic)
            .then(|| actual_start.checked_add(u64::from(activity.scheduled_duration_seconds?)))
            .flatten(),
        delivery_millimeters_per_hour: nominal_delivery_millimeters_per_hour(head),
    };
    assert!(
        valid(&checkpoint),
        "Invalid watering checkpoint; watering must remain stopped"
    );
    Some(checkpoint)
}

pub(super) fn previous_build_checkpoint(
    activity: &SprinklerWateringActivityV1,
    events: &[SprinklerWaterEventV1],
    head: SprinklerHeadTypeV1,
) -> Option<SprinklerRestartAccountingV1> {
    let start = activity.actual_starts_at?;
    let accounted_through = start.checked_add(u64::from(
        activity.actual_duration_seconds.unwrap_or_default(),
    ))?;
    from_activity(
        activity,
        events,
        accounted_through,
        activity.watering_percent,
        head,
    )
}

fn extend_segment(
    checkpoint: &SprinklerRestartAccountingV1,
    through: u64,
) -> Option<SprinklerWaterEventV1> {
    let SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        watering_percent,
        applied_water_millimeters,
        ..
    } = checkpoint.segment
    else {
        return None;
    };
    let added_seconds = through.checked_sub(checkpoint.accounted_through)?;
    let duration_seconds = u32::try_from(through.checked_sub(starts_at)?).ok()?;
    let applied_water_millimeters = applied_water_millimeters
        + checkpoint.delivery_millimeters_per_hour * added_seconds as f32 / 3_600.0;
    valid_nonnegative(applied_water_millimeters).then_some(SprinklerWaterEventV1::IrrigationV1 {
        starts_at,
        duration_seconds,
        watering_percent,
        applied_water_millimeters,
    })
}

pub(super) fn projected_segment(
    checkpoint: &SprinklerRestartAccountingV1,
) -> SprinklerWaterEventV1 {
    let end = if checkpoint.activity.outcome == SprinklerWateringOutcomeV1::Running {
        checkpoint
            .expected_ends_at
            .unwrap_or(checkpoint.accounted_through)
    } else {
        checkpoint.accounted_through
    };
    extend_segment(checkpoint, end.max(checkpoint.accounted_through))
        .expect("Watering interval exceeds representable accounting bounds")
}

pub(super) fn replace_segment(
    events: &mut Vec<SprinklerWaterEventV1>,
    segment: &SprinklerWaterEventV1,
) {
    let index = water_event_index(segment).expect("Invalid watering segment index");
    events.retain(|event| water_event_index(event) != Some(index));
    if segment.duration_seconds() > 0 {
        events.push(segment.clone());
    }
    sort_water_events(events);
}

pub(super) fn confirmed(
    checkpoint: &SprinklerRestartAccountingV1,
    is_open: bool,
    observed_at: u64,
) -> Option<SprinklerRestartAccountingV1> {
    if !valid(checkpoint) || observed_at < checkpoint.accounted_through {
        return None;
    }
    // Open/Closed has no historical transition time or run identifier. Across
    // a brief outage assume continuity between open reports. A first Closed
    // report bounds the stop by the earlier of its receipt and the timed end.
    let through = if is_open {
        observed_at
    } else {
        observed_at.min(checkpoint.expected_ends_at.unwrap_or(observed_at))
    }
    .max(checkpoint.accounted_through);
    let added = u32::try_from(through - checkpoint.accounted_through).ok()?;
    let mut next = checkpoint.clone();
    next.segment = extend_segment(checkpoint, through)?;
    next.accounted_through = through;
    let duration = next
        .activity
        .actual_duration_seconds
        .unwrap_or_default()
        .checked_add(added)?;
    next.activity.actual_duration_seconds = (duration > 0).then_some(duration);
    next.activity.applied_water_millimeters = (duration > 0).then_some(
        next.activity.applied_water_millimeters.unwrap_or_default()
            + next.delivery_millimeters_per_hour * added as f32 / 3_600.0,
    );
    next.activity.updated_at = observed_at;
    if !is_open {
        next.activity.outcome = SprinklerWateringOutcomeV1::Completed;
        if next.expected_ends_at.is_some_and(|end| observed_at < end) {
            mark_early_stop(&mut next.activity);
        }
    }
    valid(&next).then_some(next)
}

pub(super) fn mark_early_stop(activity: &mut SprinklerWateringActivityV1) {
    // Preserve a known reason such as freezing weather or winterization.
    if matches!(
        activity.reason,
        SprinklerWateringReasonV1::SmartSchedule | SprinklerWateringReasonV1::LegacyUnknown
    ) {
        activity.reason = SprinklerWateringReasonV1::StoppedEarly;
    }
}

pub(super) fn write(valve: LibertasDevice, checkpoint: Option<SprinklerRestartAccountingV1>) {
    assert!(
        checkpoint.as_ref().is_none_or(valid),
        "Invalid restart checkpoint"
    );
    let value = SprinklerData::WateringRestartV1 { checkpoint };
    libertas_data_write_single(WATERING_RESTART_RESOURCE, &zone_key(valve), &value);
    assert_eq!(
        libertas_data_read_single::<SprinklerData>(WATERING_RESTART_RESOURCE, &zone_key(valve)),
        Some(value),
        "Watering restart checkpoint write was not confirmed"
    );
}

pub(super) fn load(valve: LibertasDevice) -> Option<SprinklerRestartAccountingV1> {
    match libertas_data_read_single(WATERING_RESTART_RESOURCE, &zone_key(valve)) {
        Some(SprinklerData::WateringRestartV1 { checkpoint }) => {
            assert!(
                checkpoint.as_ref().is_none_or(valid),
                "Invalid saved restart accounting"
            );
            checkpoint
        }
        None => {
            write(valve, None);
            None
        }
        Some(_) => panic!("Unexpected restart accounting data; automatic watering remains stopped"),
    }
}

fn write_segment(valve: LibertasDevice, segment: SprinklerWaterEventV1) {
    let database = libertas_data_open_indexed(WATER_EVENTS_RESOURCE, &zone_key(valve));
    let index = water_event_index(&segment).expect("Invalid watering index");
    if segment.duration_seconds() == 0 {
        libertas_data_remove_indexed_records(database.handle, index, index);
        assert!(
            libertas_data_read_indexed::<SprinklerData>(database.handle, index)
                .is_none_or(|saved| saved.index != index),
            "Empty watering segment removal was not confirmed"
        );
    } else {
        let value = SprinklerData::WaterEventV1 { event: segment };
        libertas_data_write_indexed(database.handle, index, &value);
        assert!(
            libertas_data_read_indexed::<SprinklerData>(database.handle, index)
                .is_some_and(|saved| saved.index == index && saved.data == value),
            "Watering segment write was not confirmed"
        );
    }
}

fn write_activity(valve: LibertasDevice, checkpoint: &SprinklerRestartAccountingV1) {
    persist_watering_activity(valve, &checkpoint.activity);
    let expected = watering_activity_state(checkpoint.activity.clone());
    assert!(
        matches!(libertas_data_read_single::<SprinklerData>(WATERING_ACTIVITY_STATE_RESOURCE, &zone_key(valve)),
            Some(SprinklerData::WateringActivityStateV2 { state }) if state == expected),
        "Watering activity checkpoint write was not confirmed"
    );
}

trait CheckpointWriter {
    fn journal(&mut self, checkpoint: Option<&SprinklerRestartAccountingV1>) -> Result<(), ()>;
    fn activity(&mut self, checkpoint: &SprinklerRestartAccountingV1) -> Result<(), ()>;
    fn segment(&mut self, segment: &SprinklerWaterEventV1) -> Result<(), ()>;
}

fn commit(
    writer: &mut impl CheckpointWriter,
    checkpoint: &SprinklerRestartAccountingV1,
) -> Result<(), ()> {
    if !valid(checkpoint) {
        return Err(());
    }
    // The journal is authoritative if any later write or its readback fails.
    writer.journal(Some(checkpoint))?;
    writer.activity(checkpoint)?;
    writer.segment(&checkpoint.segment)?;
    if checkpoint.activity.outcome != SprinklerWateringOutcomeV1::Running {
        writer.journal(None)?;
    }
    Ok(())
}

struct HostWriter(LibertasDevice);

impl CheckpointWriter for HostWriter {
    fn journal(&mut self, checkpoint: Option<&SprinklerRestartAccountingV1>) -> Result<(), ()> {
        write(self.0, checkpoint.cloned());
        Ok(())
    }
    fn activity(&mut self, checkpoint: &SprinklerRestartAccountingV1) -> Result<(), ()> {
        write_activity(self.0, checkpoint);
        Ok(())
    }
    fn segment(&mut self, segment: &SprinklerWaterEventV1) -> Result<(), ()> {
        write_segment(self.0, segment.clone());
        Ok(())
    }
}

pub(super) fn persist(valve: LibertasDevice, checkpoint: &SprinklerRestartAccountingV1) {
    commit(&mut HostWriter(valve), checkpoint).expect("Watering accounting commit failed");
}

pub(super) fn restore(
    valve: LibertasDevice,
    events: &mut Vec<SprinklerWaterEventV1>,
    checkpoint: &SprinklerRestartAccountingV1,
) {
    // Runtime uses the confirmed prefix until a fresh valve report extends it.
    // The journal retains the timed end across another restart.
    replace_segment(events, &checkpoint.segment);
    persist(valve, checkpoint);
}

pub(super) fn fold_time(zone: &ZoneRuntime, now: u64) -> u64 {
    zone.restart_accounting
        .as_ref()
        .map(|saved| saved.accounted_through)
        .or_else(|| {
            zone.valve_is_open
                .then_some(zone.accounted_at_utc)
                .flatten()
        })
        .map_or(now, |through| now.min(through))
}

struct ShutdownJob {
    shared: Rc<RefCell<ControllerState>>,
    next_zone: usize,
}

fn prepare_shutdown_zone(
    zone: &mut ZoneRuntime,
    now_ticks: u64,
    now: Option<LibertasDateTime>,
    site: Option<SprinklerWeatherLocationV1>,
) -> (
    Option<SprinklerRestartAccountingV1>,
    ZonePersistenceChange,
    Vec<SprinklerDailyReportV1>,
) {
    let previous_memory = zone.memory.clone();
    let previous_events = zone.water_events.clone();
    // A second upgrade before the first valve report preserves the original
    // checkpoint; shutdown is not another observation of the valve.
    let checkpoint = if let Some(saved) = &zone.restart_accounting {
        Some(saved.clone())
    } else {
        account_open_zone(zone, now_ticks, now, site);
        capture(zone)
    };
    (
        checkpoint,
        zone_persistence_change(zone, previous_memory, previous_events),
        core::mem::take(&mut zone.finalized_daily_reports),
    )
}

fn shutdown_step(timer: u32, _now: u64, context: &mut Box<dyn Any>) {
    let job = context.downcast_mut::<ShutdownJob>().unwrap();
    let now_ticks = libertas_get_sys_ticks();
    let now = utc_seconds();
    let prepared = {
        let mut state = job.shared.borrow_mut();
        let site = state.site_location;
        state.zones.get_mut(job.next_zone).map(|zone| {
            let (checkpoint, change, reports) = prepare_shutdown_zone(zone, now_ticks, now, site);
            (zone.configuration.valve, checkpoint, change, reports)
        })
    };
    if let Some((valve, checkpoint, change, reports)) = prepared {
        if let Some(checkpoint) = &checkpoint {
            write(valve, Some(checkpoint.clone()));
        }
        persist_daily_reports(valve, &reports);
        change.submit();
        if let Some(checkpoint) = checkpoint {
            persist(valve, &checkpoint);
            write_segment(valve, projected_segment(&checkpoint));
        }
        job.next_zone += 1;
        libertas_timer_update_interval(
            timer,
            libertas_get_sys_ticks().saturating_add(YIELD_MICROSECONDS),
        );
    } else {
        libertas_timer_cancel(timer);
        libertas_shutdown_complete();
    }
}

pub(super) fn register_shutdown(shared: &Rc<RefCell<ControllerState>>) {
    libertas_register_shutdown_handler(
        |context| {
            let shared = context
                .downcast_mut::<Rc<RefCell<ControllerState>>>()
                .unwrap();
            if shared.borrow().shutting_down {
                return;
            }
            shared.borrow_mut().shutting_down = true;
            libertas_timer_new_interval(
                libertas_get_sys_ticks().saturating_add(YIELD_MICROSECONDS),
                shutdown_step,
                Box::new(ShutdownJob {
                    shared: Rc::clone(shared),
                    next_zone: 0,
                }),
            );
        },
        Box::new(Rc::clone(shared)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const START: u64 = 20_000 * SECONDS_PER_DAY + 12 * 3_600;

    fn checkpoint() -> SprinklerRestartAccountingV1 {
        let mut activity = crate::tests::completed_report_activity(START);
        activity.outcome = SprinklerWateringOutcomeV1::Running;
        activity.reason = SprinklerWateringReasonV1::SmartSchedule;
        activity.scheduled_duration_seconds = Some(900);
        activity.actual_duration_seconds = Some(300);
        activity.applied_water_millimeters = Some(1.0);
        activity.updated_at = START + 300;
        SprinklerRestartAccountingV1 {
            activity,
            accounted_through: START + 300,
            segment: SprinklerWaterEventV1::IrrigationV1 {
                starts_at: START,
                duration_seconds: 300,
                watering_percent: 100,
                applied_water_millimeters: 1.0,
            },
            expected_ends_at: Some(START + 900),
            delivery_millimeters_per_hour: 12.0,
        }
    }

    #[test]
    fn shutdown_checkpoints_the_tail_without_stopping_or_restarting_the_valve() {
        let saved = checkpoint();
        let mut zone = crate::tests::runtime(default_memory(START));
        zone.water_events = vec![saved.segment.clone()];
        zone.current_activity = Some(saved.activity);
        zone.valve_is_open = true;
        zone.valve_opened_automatically = true;
        zone.accounted_at_utc = Some(START + 300);
        zone.accounted_at_ticks = Some(10 * MICROSECONDS_PER_SECOND);
        let (checkpoint, change, _) = prepare_shutdown_zone(
            &mut zone,
            40 * MICROSECONDS_PER_SECOND,
            Some(START + 330),
            None,
        );
        let checkpoint = checkpoint.unwrap();
        assert!(zone.valve_is_open);
        assert!(zone.pending_command.is_none());
        assert_eq!(checkpoint.activity.actual_duration_seconds, Some(330));
        assert_eq!(checkpoint.segment.duration_seconds(), 330);
        assert_eq!(change.water_events.len(), 1);
        assert_eq!(projected_segment(&checkpoint).duration_seconds(), 900);
        // Another upgrade while recovery is waiting cannot fabricate a newer
        // observation or lose the original confirmed amount.
        zone.valve_is_open = false;
        zone.restart_accounting = Some(checkpoint.clone());
        let (repeated, _, _) = prepare_shutdown_zone(
            &mut zone,
            70 * MICROSECONDS_PER_SECOND,
            Some(START + 360),
            None,
        );
        assert_eq!(repeated, Some(checkpoint));
    }

    #[test]
    fn upgrade_keeps_one_run_and_accounts_the_gap_before_resuming_monotonic_time() {
        let saved = checkpoint();
        let mut events = vec![projected_segment(&saved)];
        assert_eq!(events[0].duration_seconds(), 900);
        replace_segment(&mut events, &saved.segment);
        let confirmed = confirmed(&saved, true, START + 330).unwrap();
        replace_segment(&mut events, &confirmed.segment);
        let mut zone = crate::tests::runtime(default_memory(START));
        zone.water_events = events;
        zone.current_activity = Some(confirmed.activity);
        zone.valve_is_open = true;
        zone.valve_opened_automatically = true;
        zone.accounted_at_utc = Some(START + 330);
        zone.accounted_at_ticks = Some(10 * MICROSECONDS_PER_SECOND);
        assert!(account_open_zone(
            &mut zone,
            70 * MICROSECONDS_PER_SECOND,
            Some(START + 390),
            None
        ));
        assert_eq!(zone.water_events.len(), 1);
        assert_eq!(zone.water_events[0].duration_seconds(), 390);
        let activity = zone.current_activity.as_ref().unwrap();
        assert_eq!(activity.activity_index, saved.activity.activity_index);
        assert_eq!(activity.actual_duration_seconds, Some(390));
        assert!((activity.applied_water_millimeters.unwrap() - 1.3).abs() < 0.0001);
        assert_eq!(
            capture(&zone).unwrap().expected_ends_at,
            saved.expected_ends_at
        );
    }

    #[test]
    fn first_closed_report_shortens_the_full_period_in_place() {
        let saved = checkpoint();
        let mut events = vec![projected_segment(&saved)];
        let closed = confirmed(&saved, false, START + 360).unwrap();
        replace_segment(&mut events, &closed.segment);
        assert_eq!(events.len(), 1);
        assert_eq!(
            water_event_index(&events[0]),
            water_event_index(&saved.segment)
        );
        assert_eq!(events[0].duration_seconds(), 360);
        assert_eq!(closed.activity.actual_duration_seconds, Some(360));
        assert!((closed.activity.applied_water_millimeters.unwrap() - 1.2).abs() < 0.0001);
        assert_eq!(
            closed.activity.reason,
            SprinklerWateringReasonV1::StoppedEarly
        );
        assert_eq!(closed.activity.scheduled_duration_seconds, Some(900));
        assert_eq!(
            closed.activity.outcome,
            SprinklerWateringOutcomeV1::Completed
        );
    }

    #[test]
    fn power_loss_uses_the_normal_running_checkpoint_without_a_shutdown_callback() {
        let saved = checkpoint();
        let mut store = MemoryWriter::default();
        commit(&mut store, &saved).unwrap();
        // Both Hub and valve stop; their first post-boot report is Closed two
        // minutes after the last checkpoint. The outage uncertainty is bounded
        // by this report, not the remaining ten-minute reservation.
        let closed = confirmed(store.journal.as_ref().unwrap(), false, START + 420).unwrap();
        commit(&mut store, &closed).unwrap();
        assert!(store.journal.is_none());
        assert_eq!(store.events.len(), 1);
        assert_eq!(store.events[0].duration_seconds(), 420);
        assert_eq!(store.activity.unwrap().actual_duration_seconds, Some(420));
    }

    #[test]
    fn closed_after_the_timed_end_keeps_exactly_the_full_period() {
        let closed = confirmed(&checkpoint(), false, START + 1_500).unwrap();
        assert_eq!(closed.segment.duration_seconds(), 900);
        assert_eq!(closed.activity.actual_duration_seconds, Some(900));
        assert!((closed.activity.applied_water_millimeters.unwrap() - 3.0).abs() < 0.0001);
        assert_eq!(
            closed.activity.reason,
            SprinklerWateringReasonV1::SmartSchedule
        );
    }

    #[test]
    fn early_close_preserves_a_known_safety_reason() {
        let mut saved = checkpoint();
        saved.activity.reason = SprinklerWateringReasonV1::FreezingWeather;
        let closed = confirmed(&saved, false, START + 360).unwrap();
        assert_eq!(
            closed.activity.reason,
            SprinklerWateringReasonV1::FreezingWeather
        );
        assert_eq!(closed.activity.actual_duration_seconds, Some(360));
    }

    #[test]
    fn repeated_upgrades_and_duplicate_confirmation_do_not_add_water_twice() {
        let first = confirmed(&checkpoint(), true, START + 330).unwrap();
        let second = confirmed(&first, true, START + 360).unwrap();
        assert_eq!(second.activity.actual_duration_seconds, Some(360));
        assert_eq!(confirmed(&second, true, START + 360), Some(second.clone()));
        let mut events = vec![projected_segment(&first)];
        for _ in 0..3 {
            replace_segment(&mut events, &second.segment);
        }
        assert_eq!(events, vec![second.segment]);
    }

    #[test]
    fn reconciliation_preserves_an_earlier_merged_run_prefix() {
        let mut saved = checkpoint();
        saved.segment = SprinklerWaterEventV1::IrrigationV1 {
            starts_at: START - 60,
            duration_seconds: 360,
            watering_percent: 100,
            applied_water_millimeters: 1.2,
        };
        let closed = confirmed(&saved, false, START + 360).unwrap();
        assert_eq!(closed.segment.duration_seconds(), 420);
        assert_eq!(closed.activity.actual_duration_seconds, Some(360));
        let SprinklerWaterEventV1::IrrigationV1 {
            applied_water_millimeters,
            ..
        } = closed.segment
        else {
            unreachable!()
        };
        assert!((applied_water_millimeters - 1.4).abs() < 0.0001);
    }

    #[test]
    fn midnight_restart_preserves_the_unreconciled_day_under_pruning_pressure() {
        let saved = checkpoint();
        let mut zone = crate::tests::runtime(default_memory(START));
        zone.restart_accounting = Some(saved.clone());
        let tomorrow = utc_day_start(START) + SECONDS_PER_DAY;
        zone.water_events = (0..2_049)
            .map(|offset| SprinklerWaterEventV1::IrrigationV1 {
                starts_at: START + offset,
                duration_seconds: 1,
                watering_percent: 100,
                applied_water_millimeters: 0.01,
            })
            .collect();
        let fold_at = fold_time(&zone, tomorrow + 120);
        let reports = prune_water_events(
            &mut zone.memory,
            &mut zone.water_events,
            &mut zone.modeled_weather_gaps,
            &zone.configuration,
            None,
            fold_at,
        );
        assert!(reports.is_empty());
        assert_eq!(zone.memory.balance_baseline_at, START);
        assert_eq!(zone.water_events.len(), 2_049);
    }

    #[test]
    fn unobserved_pending_commands_cannot_create_delivered_water() {
        let mut zone = crate::tests::runtime(default_memory(START));
        let mut activity = checkpoint().activity;
        activity.outcome = SprinklerWateringOutcomeV1::CommandPending;
        activity.actual_starts_at = None;
        activity.actual_duration_seconds = None;
        activity.applied_water_millimeters = None;
        zone.current_activity = Some(activity);
        assert!(capture(&zone).is_none());
        assert!(zone.water_events.is_empty());
    }

    #[test]
    fn a_first_open_checkpoint_covers_power_loss_before_the_first_minute_tick() {
        let mut saved = checkpoint();
        saved.accounted_through = START;
        if let SprinklerWaterEventV1::IrrigationV1 {
            duration_seconds,
            applied_water_millimeters,
            ..
        } = &mut saved.segment
        {
            *duration_seconds = 0;
            *applied_water_millimeters = 0.0;
        }
        saved.activity.actual_duration_seconds = None;
        saved.activity.applied_water_millimeters = None;
        saved.activity.updated_at = START;
        assert!(valid(&saved));
        let closed = confirmed(&saved, false, START + 120).unwrap();
        assert_eq!(closed.segment.duration_seconds(), 120);
        assert_eq!(closed.activity.actual_duration_seconds, Some(120));
    }

    #[test]
    fn manual_runs_keep_their_origin_and_have_no_invented_timed_end() {
        let mut saved = checkpoint();
        saved.activity.origin = SprinklerWateringOriginV1::Manual;
        saved.activity.activity_index =
            watering_activity_index(START, saved.activity.origin, 0).unwrap();
        saved.activity.reason = SprinklerWateringReasonV1::ManualOperation;
        saved.expected_ends_at = None;
        let resumed = confirmed(&saved, true, START + 420).unwrap();
        assert_eq!(resumed.activity.origin, SprinklerWateringOriginV1::Manual);
        assert_eq!(resumed.activity.actual_duration_seconds, Some(420));
        assert_eq!(projected_segment(&resumed), resumed.segment);
    }

    #[test]
    fn invalid_checkpoints_and_backward_time_are_rejected() {
        let saved = checkpoint();
        assert!(confirmed(&saved, true, START + 299).is_none());
        let mut invalid = saved.clone();
        invalid.accounted_through += 1;
        assert!(!valid(&invalid));
        invalid = saved.clone();
        invalid.delivery_millimeters_per_hour = f32::NAN;
        assert!(!valid(&invalid));
        invalid = saved;
        invalid.expected_ends_at = Some(START - 1);
        assert!(!valid(&invalid));
    }

    #[test]
    fn appended_checkpoint_and_early_stop_reason_have_stable_avro_encoding() {
        for saved in [None, Some(checkpoint())] {
            let value = SprinklerData::WateringRestartV1 { checkpoint: saved };
            let bytes = value.to_avro();
            assert_eq!(bytes[0], 26);
            assert_eq!(SprinklerData::from_avro(&bytes), Ok(value));
            assert!(SprinklerData::from_avro(&bytes[..bytes.len() - 1]).is_err());
        }
        assert_eq!(SprinklerWateringReasonV1::StoppedEarly.to_avro(), vec![32]);
        let saved = checkpoint();
        assert_eq!(
            SprinklerRestartAccountingV1::from_avro(&saved.to_avro()),
            Ok(saved)
        );
    }

    #[derive(Default)]
    struct MemoryWriter {
        journal: Option<SprinklerRestartAccountingV1>,
        activity: Option<SprinklerWateringActivityV1>,
        events: Vec<SprinklerWaterEventV1>,
        writes: usize,
        fail_after_write: Option<usize>,
    }

    impl MemoryWriter {
        fn confirmed_write(&mut self) -> Result<(), ()> {
            self.writes += 1;
            if self.fail_after_write == Some(self.writes) {
                Err(())
            } else {
                Ok(())
            }
        }
    }

    impl CheckpointWriter for MemoryWriter {
        fn journal(&mut self, checkpoint: Option<&SprinklerRestartAccountingV1>) -> Result<(), ()> {
            self.journal = checkpoint.cloned();
            self.confirmed_write()
        }
        fn activity(&mut self, checkpoint: &SprinklerRestartAccountingV1) -> Result<(), ()> {
            self.activity = Some(checkpoint.activity.clone());
            self.confirmed_write()
        }
        fn segment(&mut self, segment: &SprinklerWaterEventV1) -> Result<(), ()> {
            replace_segment(&mut self.events, segment);
            self.confirmed_write()
        }
    }

    #[test]
    fn interrupted_close_commits_replay_the_known_stop_without_extending_it() {
        let original = checkpoint();
        let closed = confirmed(&original, false, START + 360).unwrap();
        for failure in 1..=4 {
            let mut writer = MemoryWriter {
                journal: Some(original.clone()),
                activity: Some(original.activity.clone()),
                events: vec![projected_segment(&original)],
                fail_after_write: Some(failure),
                ..MemoryWriter::default()
            };
            assert_eq!(commit(&mut writer, &closed), Err(()));
            if failure < 4 {
                assert_eq!(writer.journal, Some(closed.clone()));
            }
            writer.fail_after_write = None;
            if let Some(replay) = writer.journal.clone() {
                commit(&mut writer, &replay).unwrap();
            }
            assert!(writer.journal.is_none());
            assert_eq!(writer.events, vec![closed.segment.clone()]);
            assert_eq!(writer.activity, Some(closed.activity.clone()));
        }
    }
}
