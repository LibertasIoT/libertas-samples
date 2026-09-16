//! Recover the complete date-indexed ledger before enabling the controller.

use super::*;

const PAGE_SIZE: usize = 64;
const YIELD_MICROSECONDS: u64 = 5_000;

trait Ledger {
    fn bounds(&self) -> (u64, i64, i64);
    fn read_page(&self, index: i64, direction: IndexDirection) -> Vec<IndexedData<SprinklerData>>;
    fn remove(&mut self, index: i64);
}

struct HostLedger {
    valve: LibertasDevice,
    handle: libertas::LibertasDataStore,
}

impl Ledger for HostLedger {
    fn bounds(&self) -> (u64, i64, i64) {
        let stat = libertas_data_open_indexed(WATER_EVENTS_RESOURCE, &zone_key(self.valve));
        (stat.count, stat.min_index, stat.max_index)
    }

    fn read_page(&self, index: i64, direction: IndexDirection) -> Vec<IndexedData<SprinklerData>> {
        let mut records = Vec::new();
        libertas_data_read_indexed_range(self.handle, index, direction, PAGE_SIZE, &mut records);
        records
    }

    fn remove(&mut self, index: i64) {
        libertas_data_remove_indexed_records(self.handle, index, index);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryError {
    InvalidDate,
    InvalidBounds,
    IncompleteRead,
    InvalidRecord(i64),
    DatabaseChanged,
    UnconfirmedRemoval,
}

#[derive(Clone, Copy)]
enum Cursor {
    Before(i64),
    After(i64),
    Complete,
}

fn date_index(date: LibertasDateTime) -> Result<i64, RecoveryError> {
    i64::try_from(date)
        .ok()
        .and_then(|date| date.checked_mul(WATER_EVENT_INDEX_KIND_COUNT))
        .ok_or(RecoveryError::InvalidDate)
}

struct LedgerScan {
    baseline: LibertasDateTime,
    split: i64,
    overlap_floor: i64,
    original: (u64, i64, i64),
    cursor: Cursor,
    scanned: u64,
    finished: bool,
    events: Vec<SprinklerWaterEventV1>,
    folded: Vec<i64>,
    removed: u64,
}

impl LedgerScan {
    fn new(ledger: &impl Ledger, baseline: LibertasDateTime) -> Result<Self, RecoveryError> {
        let split = date_index(baseline)?;
        // Durations are u32 seconds. Earlier intervals cannot overlap the
        // baseline. Do not assume just the nearest predecessor is sufficient:
        // a long manual run can precede many shorter weather periods.
        let overlap_floor = date_index(baseline.saturating_sub(u64::from(u32::MAX)))?;
        let original = ledger.bounds();
        if original.0 > 0 && (original.1 < 0 || original.1 > original.2) {
            return Err(RecoveryError::InvalidBounds);
        }
        let cursor = if original.0 == 0 {
            Cursor::Complete
        } else if original.1 < split {
            Cursor::Before(split - 1)
        } else {
            Cursor::After(split)
        };
        Ok(Self {
            baseline,
            split,
            overlap_floor,
            original,
            cursor,
            scanned: 0,
            finished: false,
            events: Vec::new(),
            folded: Vec::new(),
            removed: 0,
        })
    }

    // One indexed seek/page per callback, with no cap on the total record
    // count. Until the complete range is validated, this does not write data.
    fn step(&mut self, ledger: &impl Ledger) -> Result<bool, RecoveryError> {
        if self.finished {
            return Ok(true);
        }
        let (next, direction, before) = match self.cursor {
            Cursor::Before(next) if next >= self.overlap_floor && next >= self.original.1 => {
                (next, IndexDirection::Below, true)
            }
            Cursor::Before(_) => {
                self.cursor = Cursor::After(self.split);
                return Ok(false);
            }
            Cursor::After(next) if self.original.0 > 0 && next <= self.original.2 => {
                (next, IndexDirection::Above, false)
            }
            Cursor::After(_) | Cursor::Complete => {
                let current = ledger.bounds();
                if current.0 != self.original.0
                    || (current.0 > 0 && current != self.original)
                    || (self.original.0 > 0
                        && self.overlap_floor <= self.original.1
                        && self.scanned != self.original.0)
                {
                    return Err(RecoveryError::DatabaseChanged);
                }
                sort_water_events(&mut self.events);
                self.finished = true;
                return Ok(true);
            }
        };
        let records = ledger.read_page(next, direction);
        if records.is_empty() || records.len() > PAGE_SIZE {
            return Err(RecoveryError::IncompleteRead);
        }
        let mut previous = None;
        for record in records {
            if record.index < self.original.1
                || record.index > self.original.2
                || (before && (record.index > next || previous.is_some_and(|p| p <= record.index)))
                || (!before && (record.index < next || previous.is_some_and(|p| p >= record.index)))
            {
                return Err(RecoveryError::IncompleteRead);
            }
            previous = Some(record.index);
            if before && record.index < self.overlap_floor {
                self.cursor = Cursor::After(self.split);
                return Ok(false);
            }
            self.scanned += 1;
            let SprinklerData::WaterEventV1 { event } = record.data else {
                return Err(RecoveryError::InvalidRecord(record.index));
            };
            if water_event_index(&event) != Some(record.index) || !valid_water_event(&event) {
                return Err(RecoveryError::InvalidRecord(record.index));
            }
            if event.ends_at().is_some_and(|end| end > self.baseline) {
                self.events.push(event);
            } else {
                // This input is already covered by the persisted baseline.
                self.folded.push(record.index);
            }
            self.cursor = if before {
                if record.index <= self.overlap_floor || record.index == self.original.1 {
                    Cursor::After(self.split)
                } else {
                    Cursor::Before(record.index - 1)
                }
            } else if record.index == self.original.2 {
                Cursor::Complete
            } else {
                Cursor::After(record.index + 1)
            };
        }
        Ok(false)
    }

    fn discard_folded_page(&mut self, ledger: &mut impl Ledger) -> Result<bool, RecoveryError> {
        if !self.finished {
            return Err(RecoveryError::IncompleteRead);
        }
        for _ in 0..PAGE_SIZE {
            let Some(index) = self.folded.pop() else {
                break;
            };
            ledger.remove(index);
            self.removed += 1;
        }
        if self.folded.is_empty() {
            if ledger.bounds().0 != self.original.0.saturating_sub(self.removed) {
                return Err(RecoveryError::UnconfirmedRemoval);
            }
            return Ok(true);
        }
        Ok(false)
    }
}

pub(super) struct RecoveredZone {
    pub configuration: SprinklerZoneV1,
    pub memory: SprinklerZoneMemoryV1,
    pub water_events: Vec<SprinklerWaterEventV1>,
}

struct PendingZone {
    configuration: SprinklerZoneV1,
    memory: SprinklerZoneMemoryV1,
    ledger: HostLedger,
    scan: LedgerScan,
}

struct RecoveryJob {
    configurations: alloc::vec::IntoIter<SprinklerZoneV1>,
    current: Option<PendingZone>,
    recovered: Vec<RecoveredZone>,
    on_complete: Option<Box<dyn FnOnce(Vec<RecoveredZone>)>>,
}

fn recover_next(job: &mut RecoveryJob) -> Result<bool, RecoveryError> {
    if let Some(pending) = job.current.as_mut() {
        if !pending.scan.finished {
            pending.scan.step(&pending.ledger)?;
            return Ok(false);
        }
        if !pending.scan.discard_folded_page(&mut pending.ledger)? {
            return Ok(false);
        }
        let pending = job.current.take().unwrap();
        libertas_log(
            LogLevel::Info,
            &alloc::format!(
                "Water ledger recovery for valve {}: scanned {} records; restored {} unaccounted events",
                pending.configuration.valve,
                pending.scan.scanned,
                pending.scan.events.len(),
            ),
        );
        job.recovered.push(RecoveredZone {
            configuration: pending.configuration,
            memory: pending.memory,
            water_events: pending.scan.events,
        });
        return Ok(false);
    }
    let Some(configuration) = job.configurations.next() else {
        return Ok(true);
    };
    let memory = load_zone_memory(configuration.valve, utc_seconds().unwrap_or_default());
    let database =
        libertas_data_open_indexed(WATER_EVENTS_RESOURCE, &zone_key(configuration.valve));
    let ledger = HostLedger {
        valve: configuration.valve,
        handle: database.handle,
    };
    let scan = LedgerScan::new(&ledger, memory.balance_baseline_at)?;
    job.current = Some(PendingZone {
        configuration,
        memory,
        ledger,
        scan,
    });
    Ok(false)
}

fn recovery_timer(timer: u32, _now: u64, context: &mut Box<dyn Any>) {
    let job = context.downcast_mut::<RecoveryJob>().unwrap();
    match recover_next(job) {
        Ok(false) => {
            // Fresh ticks ensure the SDK yields between database pages.
            libertas_timer_update_interval(
                timer,
                libertas_get_sys_ticks().saturating_add(YIELD_MICROSECONDS),
            );
        }
        Ok(true) => {
            libertas_timer_cancel(timer);
            if let Some(on_complete) = job.on_complete.take() {
                on_complete(core::mem::take(&mut job.recovered));
            }
        }
        Err(error) => {
            libertas_timer_cancel(timer);
            libertas_log(
                LogLevel::Error,
                &alloc::format!(
                    "Water ledger recovery failed: {error:?}; automatic watering remains stopped",
                ),
            );
            // Never initialize a controller from partial accounting data.
            job.on_complete = None;
            job.current = None;
            job.recovered.clear();
        }
    }
}

pub(super) fn start(
    zones: Vec<SprinklerZoneV1>,
    on_complete: impl FnOnce(Vec<RecoveredZone>) + 'static,
) {
    libertas_timer_new_interval(
        libertas_get_sys_ticks().saturating_add(YIELD_MICROSECONDS),
        recovery_timer,
        Box::new(RecoveryJob {
            configurations: zones.into_iter(),
            current: None,
            recovered: Vec::new(),
            on_complete: Some(Box::new(on_complete)),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{collections::BTreeMap, vec};
    use core::cell::{Cell, RefCell};

    const DAY: u64 = 20_000 * SECONDS_PER_DAY;

    #[derive(Default)]
    struct MemoryLedger {
        rows: BTreeMap<i64, SprinklerData>,
        reads: RefCell<Vec<(i64, bool)>>,
        fail_after: Option<usize>,
        ignore_removals: bool,
        removed: Vec<i64>,
        empty_bounds_counter: Cell<i64>,
    }

    impl Ledger for MemoryLedger {
        fn bounds(&self) -> (u64, i64, i64) {
            // The SDK makes min/max undefined for an empty database.
            let undefined = self.empty_bounds_counter.get();
            self.empty_bounds_counter.set(undefined + 1);
            (
                self.rows.len() as u64,
                self.rows
                    .first_key_value()
                    .map_or(undefined, |(&key, _)| key),
                self.rows
                    .last_key_value()
                    .map_or(undefined, |(&key, _)| key),
            )
        }

        fn read_page(
            &self,
            index: i64,
            direction: IndexDirection,
        ) -> Vec<IndexedData<SprinklerData>> {
            if self
                .fail_after
                .is_some_and(|limit| self.reads.borrow().len() >= limit)
            {
                return Vec::new();
            }
            let below = matches!(direction, IndexDirection::Below);
            self.reads.borrow_mut().push((index, below));
            let rows: Box<dyn Iterator<Item = (&i64, &SprinklerData)>> = if below {
                Box::new(self.rows.range(..=index).rev())
            } else {
                Box::new(self.rows.range(index..))
            };
            rows.take(PAGE_SIZE)
                .map(|(&index, data)| IndexedData {
                    index,
                    data: data.clone(),
                })
                .collect()
        }

        fn remove(&mut self, index: i64) {
            if !self.ignore_removals {
                self.rows.remove(&index);
            }
            self.removed.push(index);
        }
    }

    fn irrigation(starts_at: u64, duration_seconds: u32) -> SprinklerWaterEventV1 {
        SprinklerWaterEventV1::IrrigationV1 {
            starts_at,
            duration_seconds,
            watering_percent: 100,
            applied_water_millimeters: 0.01,
        }
    }

    fn insert(ledger: &mut MemoryLedger, event: SprinklerWaterEventV1) {
        ledger.rows.insert(
            water_event_index(&event).unwrap(),
            SprinklerData::WaterEventV1 { event },
        );
    }

    fn finish(scan: &mut LedgerScan, ledger: &MemoryLedger) -> Result<(), RecoveryError> {
        loop {
            let reads_before = ledger.reads.borrow().len();
            let complete = scan.step(ledger)?;
            assert!(ledger.reads.borrow().len() <= reads_before + 1);
            if complete {
                return Ok(());
            }
        }
    }

    #[test]
    fn date_seek_reads_every_record_beyond_the_old_1024_limit() {
        let mut ledger = MemoryLedger::default();
        let expected: Vec<_> = (0..2_049)
            .map(|second| irrigation(DAY + second * 2, 1))
            .collect();
        for event in &expected {
            insert(&mut ledger, event.clone());
        }
        let unchanged = ledger.rows.clone();
        let mut scan = LedgerScan::new(&ledger, DAY).unwrap();
        assert!(!scan.step(&ledger).unwrap());
        assert_eq!(
            ledger.reads.borrow().as_slice(),
            &[(date_index(DAY).unwrap(), false)]
        );
        assert_eq!(scan.events.len(), PAGE_SIZE);
        finish(&mut scan, &ledger).unwrap();
        assert_eq!(scan.scanned, 2_049);
        assert_eq!(scan.events, expected);
        assert_eq!(ledger.rows, unchanged);
        assert!(scan.discard_folded_page(&mut ledger).unwrap());
        assert!(ledger.removed.is_empty());
    }

    #[test]
    fn date_seek_keeps_long_overlap_before_many_shorter_folded_periods() {
        let mut ledger = MemoryLedger::default();
        let long_run = irrigation(DAY - 3 * SECONDS_PER_DAY, 4 * SECONDS_PER_DAY as u32);
        insert(&mut ledger, long_run.clone());
        for second in 0..1_200 {
            insert(
                &mut ledger,
                SprinklerWaterEventV1::WeatherV1 {
                    starts_at: DAY - 10_000 + second * 2,
                    duration_seconds: 1,
                    precipitation_millimeters: 0.0,
                    reference_evapotranspiration_millimeters: 0.0,
                },
            );
        }
        let exact_boundary = irrigation(DAY, 60);
        insert(&mut ledger, exact_boundary.clone());
        let mut scan = LedgerScan::new(&ledger, DAY).unwrap();
        finish(&mut scan, &ledger).unwrap();
        assert_eq!(
            ledger.reads.borrow()[0],
            (date_index(DAY).unwrap() - 1, true)
        );
        assert_eq!(scan.events, vec![long_run.clone(), exact_boundary.clone()]);
        assert_eq!(scan.folded.len(), 1_200);
        assert!(ledger.removed.is_empty());
        assert!(!scan.discard_folded_page(&mut ledger).unwrap());
        assert_eq!(ledger.removed.len(), PAGE_SIZE);
        while !scan.discard_folded_page(&mut ledger).unwrap() {}
        assert_eq!(ledger.rows.len(), 2);
        let mut restarted = LedgerScan::new(&ledger, DAY).unwrap();
        finish(&mut restarted, &ledger).unwrap();
        assert_eq!(restarted.events, vec![long_run, exact_boundary]);
        assert!(restarted.folded.is_empty());
    }

    #[test]
    fn incomplete_read_cannot_delete_inputs_or_publish_a_partial_ledger() {
        let mut ledger = MemoryLedger::default();
        for second in 0..200 {
            insert(&mut ledger, irrigation(DAY + second, 1));
        }
        ledger.fail_after = Some(1);
        let original = ledger.rows.clone();
        let mut scan = LedgerScan::new(&ledger, DAY + 100).unwrap();
        assert_eq!(
            finish(&mut scan, &ledger),
            Err(RecoveryError::IncompleteRead)
        );
        assert!(!scan.finished);
        assert_eq!(
            scan.discard_folded_page(&mut ledger),
            Err(RecoveryError::IncompleteRead)
        );
        assert_eq!(ledger.rows, original);
        assert!(ledger.removed.is_empty());
        ledger.fail_after = None;
        let mut restarted = LedgerScan::new(&ledger, DAY + 100).unwrap();
        finish(&mut restarted, &ledger).unwrap();
        assert_eq!(restarted.events.len(), 100);
    }

    #[test]
    fn invalid_unaccounted_records_stop_recovery_without_deletion() {
        let mut ledger = MemoryLedger::default();
        insert(&mut ledger, irrigation(DAY - 100, 1));
        let invalid_index = date_index(DAY).unwrap();
        ledger.rows.insert(
            invalid_index,
            SprinklerData::ActivityArchiveCleanupV1 { completed: true },
        );
        let original = ledger.rows.clone();
        let mut scan = LedgerScan::new(&ledger, DAY).unwrap();
        assert_eq!(
            finish(&mut scan, &ledger),
            Err(RecoveryError::InvalidRecord(invalid_index))
        );
        assert_eq!(ledger.rows, original);
        assert!(ledger.removed.is_empty());
    }

    #[test]
    fn changing_database_or_failed_deletion_never_completes_recovery() {
        let mut ledger = MemoryLedger::default();
        insert(&mut ledger, irrigation(DAY - 60, 60));
        let mut scan = LedgerScan::new(&ledger, DAY).unwrap();
        scan.step(&ledger).unwrap();
        insert(&mut ledger, irrigation(DAY + 10, 1));
        assert_eq!(
            finish(&mut scan, &ledger),
            Err(RecoveryError::DatabaseChanged)
        );

        let mut scan = LedgerScan::new(&ledger, DAY).unwrap();
        finish(&mut scan, &ledger).unwrap();
        ledger.ignore_removals = true;
        assert_eq!(
            scan.discard_folded_page(&mut ledger),
            Err(RecoveryError::UnconfirmedRemoval)
        );
        assert_eq!(ledger.rows.len(), 2);
    }

    #[test]
    fn empty_zero_and_maximum_index_boundaries_are_safe() {
        let ledger = MemoryLedger::default();
        let mut scan = LedgerScan::new(&ledger, 0).unwrap();
        finish(&mut scan, &ledger).unwrap();
        assert!(ledger.reads.borrow().is_empty());
        assert!(matches!(
            LedgerScan::new(&ledger, u64::MAX),
            Err(RecoveryError::InvalidDate)
        ));
        for start in [0, i64::MAX as u64 / 2] {
            let mut ledger = MemoryLedger::default();
            insert(&mut ledger, irrigation(start, 1));
            let mut scan = LedgerScan::new(&ledger, start).unwrap();
            finish(&mut scan, &ledger).unwrap();
            assert_eq!(scan.events, vec![irrigation(start, 1)]);
        }
    }
}
