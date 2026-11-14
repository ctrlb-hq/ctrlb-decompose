use std::{cell::RefCell, collections::HashMap};

use chrono::{DateTime, Utc};
use hyperloglogplus::{HyperLogLog, HyperLogLogPlus};
use sketches_ddsketch::{Config as DDSketchConfig, DDSketch};
use std::collections::hash_map::RandomState;

use crate::extraction::drain3::{TypedVariable, classify_variable};
use crate::types::{PatternID, VarType};

const CARDINALITY_CAP: usize = 10_000;
const EMBEDDED_HISTORY_CAP: usize = 64;

// --- BoundedVec: reservoir-sampled collection ---

pub struct BoundedVec<T> {
    items: Vec<T>,
    capacity: usize,
    total_seen: u64,
}

impl<T> BoundedVec<T> {
    pub fn new(capacity: usize) -> Self {
        BoundedVec {
            items: Vec::with_capacity(capacity.min(64)),
            capacity,
            total_seen: 0,
        }
    }

    pub fn push(&mut self, item: T) {
        self.total_seen += 1;
        if self.capacity == 0 {
            return;
        }
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else {
            let j = fastrand::u64(0..self.total_seen);
            if (j as usize) < self.capacity {
                self.items[j as usize] = item;
            }
        }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }
}

// --- NumericStats ---

pub struct NumericStats {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    sketch: DDSketch,
}

impl NumericStats {
    pub fn new() -> Self {
        NumericStats {
            count: 0,
            sum: 0.0,
            min: f64::MAX,
            max: f64::MIN,
            sketch: DDSketch::new(DDSketchConfig::defaults()),
        }
    }

    pub fn update(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        let _ = self.sketch.add(value);
    }

    /// Backfill a constant without doing work proportional to prior line count.
    fn repeated(value: f64, count: u64) -> Self {
        let mut stats = Self::new();
        if count == 0 {
            return stats;
        }
        stats.count = count;
        stats.sum = value * count as f64;
        stats.min = value;
        stats.max = value;
        // DDSketch has no weighted insertion. Merge powers of two instead.
        let mut block = DDSketch::new(DDSketchConfig::defaults());
        block.add(value);
        let mut remaining = count;
        while remaining != 0 {
            if remaining & 1 != 0 {
                stats
                    .sketch
                    .merge(&block)
                    .expect("identical sketch configs");
            }
            remaining >>= 1;
            if remaining != 0 {
                let copy = block.clone();
                block.merge(&copy).expect("identical sketch configs");
            }
        }
        stats
    }

    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        }
    }

    pub fn quantile(&self, q: f64) -> Option<f64> {
        self.sketch.quantile(q).ok().flatten()
    }
}

// --- CategoricalStats ---

pub struct CategoricalStats {
    pub total_count: u64,
    exact_counts: HashMap<String, u64>,
    hll: Option<RefCell<HyperLogLogPlus<String, RandomState>>>,
    capped: bool,
}

impl CategoricalStats {
    pub fn new() -> Self {
        CategoricalStats {
            total_count: 0,
            exact_counts: HashMap::new(),
            hll: None,
            capped: false,
        }
    }

    pub fn update(&mut self, value: &str) {
        self.update_with_cap(value, CARDINALITY_CAP + 1);
    }

    fn update_with_cap(&mut self, value: &str, capacity: usize) {
        self.total_count += 1;

        if self.capped {
            // Only increment existing entries; use HLL for cardinality
            if let Some(count) = self.exact_counts.get_mut(value) {
                *count += 1;
            }
            if let Some(ref mut hll) = self.hll {
                hll.get_mut().insert(&value.to_string());
            }
        } else {
            *self.exact_counts.entry(value.to_string()).or_insert(0) += 1;
            if self.exact_counts.len() >= capacity {
                self.capped = true;
                let mut hll =
                    HyperLogLogPlus::new(16, RandomState::new()).expect("HLL creation failed");
                for key in self.exact_counts.keys() {
                    hll.insert(key);
                }
                self.hll = Some(RefCell::new(hll));
            }
        }
    }

    pub fn unique_count(&self) -> u64 {
        // Counting HLL registers is expensive; do it when read, not per line.
        self.hll
            .as_ref()
            .map_or(self.exact_counts.len() as u64, |hll| {
                hll.borrow_mut().count().round() as u64
            })
    }

    /// Returns top-k entries as (value, count, percentage)
    pub fn top_k(&self, k: usize) -> Vec<(String, u64, f64)> {
        let mut entries: Vec<_> = self
            .exact_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(k);
        entries
            .into_iter()
            .map(|(k, v)| {
                let pct = if self.total_count > 0 {
                    v as f64 / self.total_count as f64 * 100.0
                } else {
                    0.0
                };
                (k, v, pct)
            })
            .collect()
    }
}

// --- VarSlotStats ---

pub struct VarSlotStats {
    pub slot_index: usize,
    pub var_type: VarType,
    pub numeric: Option<NumericStats>,
    pub categorical: CategoricalStats,
    type_votes: HashMap<VarType, u64>,
}

impl VarSlotStats {
    pub fn new(slot_index: usize) -> Self {
        VarSlotStats {
            slot_index,
            var_type: VarType::String,
            numeric: None,
            categorical: CategoricalStats::new(),
            type_votes: HashMap::new(),
        }
    }

    pub fn update(&mut self, var: &TypedVariable) {
        self.update_with_cap(var, CARDINALITY_CAP + 1);
    }

    fn update_with_cap(&mut self, var: &TypedVariable, capacity: usize) {
        // Vote on type (majority wins)
        *self.type_votes.entry(var.var_type).or_insert(0) += 1;
        self.var_type = *self.type_votes.iter().max_by_key(|(_, v)| *v).unwrap().0;

        // Always update categorical
        self.categorical.update_with_cap(&var.raw, capacity);

        // Update numeric stats for numeric types
        match var.var_type {
            VarType::Integer | VarType::Float | VarType::Duration => {
                if let Some(value) = parse_numeric_value(&var.raw) {
                    let numeric = self.numeric.get_or_insert_with(NumericStats::new);
                    numeric.update(value);
                }
            }
            _ => {}
        }
    }

    fn backfill(slot_index: usize, raw: &str, count: u64) -> Self {
        let mut slot = Self::new(slot_index);
        if count == 0 {
            return slot;
        }
        slot.var_type = classify_variable(raw);
        slot.type_votes.insert(slot.var_type, count);
        slot.categorical.total_count = count;
        slot.categorical.exact_counts.insert(raw.to_string(), count);
        if matches!(
            slot.var_type,
            VarType::Integer | VarType::Float | VarType::Duration
        ) {
            if let Some(value) = parse_numeric_value(raw) {
                slot.numeric = Some(NumericStats::repeated(value, count));
            }
        }
        slot
    }

    /// Check if this slot should be reclassified as Enum
    /// (pattern >= 50 occurrences, <= 20 unique values, top 3 cover >= 80%)
    pub fn check_enum_reclassify(&mut self) {
        if self.var_type == VarType::String
            && self.categorical.total_count >= 50
            && self.categorical.unique_count() <= 20
        {
            let top3 = self.categorical.top_k(3);
            let top3_pct: f64 = top3.iter().map(|(_, _, pct)| pct).sum();
            if top3_pct >= 80.0 {
                self.var_type = VarType::Enum;
            }
        }
    }
}

fn parse_numeric_value(raw: &str) -> Option<f64> {
    // Try direct parse
    if let Ok(v) = raw.parse::<f64>() {
        return Some(v);
    }
    // Try stripping duration suffix (normalize to milliseconds)
    let suffixes = [
        ("ns", 0.000_001),
        ("us", 0.001),
        ("µs", 0.001),
        ("ms", 1.0),
        ("s", 1000.0),
        ("m", 60_000.0),
        ("h", 3_600_000.0),
    ];
    for (suffix, multiplier) in suffixes {
        if let Some(num_str) = raw.strip_suffix(suffix) {
            if let Ok(v) = num_str.parse::<f64>() {
                return Some(v * multiplier);
            }
        }
    }
    None
}

// --- PatternStats ---

pub struct PatternStats {
    pub pattern_id: PatternID,
    pub template: String,
    pub count: u64,
    pub first_seen_line: u64,
    pub last_seen_line: u64,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub variables: Vec<VarSlotStats>,
    pub time_buckets: HashMap<i64, u64>, // key = minute since epoch
    pub example_lines: BoundedVec<String>,
    // One bounded whole-token summary per fixed token with embedded variables.
    // On widening it replaces that token's subslots, without replaying lines.
    embedded_history: Vec<Option<VarSlotStats>>,
}

impl PatternStats {
    pub fn new(pattern_id: PatternID, template: String, context_lines: usize) -> Self {
        let token_count = template.split_whitespace().count();
        PatternStats {
            pattern_id,
            template,
            count: 0,
            first_seen_line: 0,
            last_seen_line: 0,
            first_ts: None,
            last_ts: None,
            variables: Vec::new(),
            time_buckets: HashMap::new(),
            example_lines: BoundedVec::new(context_lines),
            embedded_history: (0..token_count).map(|_| None).collect(),
        }
    }

    fn reconcile_template(&mut self, template: &str) {
        if self.template == template {
            return;
        }
        let mut old_slots = std::mem::take(&mut self.variables).into_iter();
        for (position, (old, new)) in self
            .template
            .split_whitespace()
            .zip(template.split_whitespace())
            .enumerate()
        {
            let subslots = old.matches("<*>").count();
            if old != new {
                debug_assert_eq!(new, "<*>", "Drain only widens whole tokens");
                // Discard all subslots belonging to this token; never let their
                // observations leak into the new slot or its following slots.
                for _ in 0..subslots {
                    old_slots.next().expect("existing token subslot");
                }
                let slot = self.embedded_history[position].take().unwrap_or_else(|| {
                    debug_assert_eq!(subslots, 0);
                    VarSlotStats::backfill(self.variables.len(), old, self.count)
                });
                self.variables.push(slot);
            } else {
                self.variables.extend(old_slots.by_ref().take(subslots));
            }
        }
        debug_assert!(old_slots.next().is_none());
        for (index, slot) in self.variables.iter_mut().enumerate() {
            slot.slot_index = index;
        }
        self.template = template.to_string();
    }
}

// --- PatternStore ---

pub struct PatternStore {
    pub patterns: HashMap<PatternID, PatternStats>,
    pub global_line_count: u64,
    pub global_first_ts: Option<DateTime<Utc>>,
    pub global_last_ts: Option<DateTime<Utc>>,
    context_lines: usize,
}

impl PatternStore {
    pub fn new(context_lines: usize) -> Self {
        PatternStore {
            patterns: HashMap::new(),
            global_line_count: 0,
            global_first_ts: None,
            global_last_ts: None,
            context_lines,
        }
    }

    pub fn accumulate(
        &mut self,
        pattern_id: PatternID,
        template: &str,
        variables: &[TypedVariable],
        embedded_tokens: &[(usize, String)],
        timestamp: Option<DateTime<Utc>>,
        raw_line: &str,
        line_number: u64,
    ) {
        self.global_line_count += 1;

        // Update global timestamps
        if let Some(ts) = timestamp {
            match self.global_first_ts {
                None => self.global_first_ts = Some(ts),
                Some(first) if ts < first => self.global_first_ts = Some(ts),
                _ => {}
            }
            match self.global_last_ts {
                None => self.global_last_ts = Some(ts),
                Some(last) if ts > last => self.global_last_ts = Some(ts),
                _ => {}
            }
        }

        let ctx = self.context_lines;
        let stats = self
            .patterns
            .entry(pattern_id)
            .or_insert_with(|| PatternStats::new(pattern_id, template.to_string(), ctx));

        stats.reconcile_template(template);

        stats.count += 1;
        if stats.first_seen_line == 0 {
            stats.first_seen_line = line_number;
        }
        stats.last_seen_line = line_number;

        // Timestamps
        if let Some(ts) = timestamp {
            match stats.first_ts {
                None => stats.first_ts = Some(ts),
                Some(first) if ts < first => stats.first_ts = Some(ts),
                _ => {}
            }
            match stats.last_ts {
                None => stats.last_ts = Some(ts),
                Some(last) if ts > last => stats.last_ts = Some(ts),
                _ => {}
            }

            // 1-minute bucket
            let minute = ts.timestamp() / 60;
            *stats.time_buckets.entry(minute).or_insert(0) += 1;
        }

        // Variable stats
        for (i, var) in variables.iter().enumerate() {
            while stats.variables.len() <= i {
                stats
                    .variables
                    .push(VarSlotStats::new(stats.variables.len()));
            }
            stats.variables[i].update(var);
        }

        for (position, raw) in embedded_tokens {
            let history =
                stats.embedded_history[*position].get_or_insert_with(|| VarSlotStats::new(0));
            history.update_with_cap(
                &TypedVariable {
                    var_type: classify_variable(raw),
                    raw: raw.clone(),
                },
                EMBEDDED_HISTORY_CAP,
            );
        }

        // Example lines
        stats.example_lines.push(raw_line.to_string());
    }

    /// Reclassify enums; all slot reconciliation happens during accumulation.
    pub fn finalize(&mut self) {
        for stats in self.patterns.values_mut() {
            for var in &mut stats.variables {
                var.check_enum_reclassify();
            }
        }
    }

    /// Patterns sorted by count descending
    pub fn sorted_patterns(&self) -> Vec<&PatternStats> {
        let mut patterns: Vec<_> = self.patterns.values().collect();
        patterns.sort_by(|a, b| b.count.cmp(&a.count));
        patterns
    }

    /// Global time range in minutes-since-epoch
    pub fn time_range_minutes(&self) -> Option<(i64, i64)> {
        let mut min_m = i64::MAX;
        let mut max_m = i64::MIN;
        for stats in self.patterns.values() {
            for &minute in stats.time_buckets.keys() {
                min_m = min_m.min(minute);
                max_m = max_m.max(minute);
            }
        }
        if min_m <= max_m {
            Some((min_m, max_m))
        } else {
            None
        }
    }

    /// Aligned time-bucket vector for a pattern
    pub fn time_bucket_vector(&self, pattern: &PatternStats) -> Vec<u64> {
        if let Some((min_m, max_m)) = self.time_range_minutes() {
            let len = (max_m - min_m + 1) as usize;
            let mut vec = vec![0u64; len];
            for (&minute, &count) in &pattern.time_buckets {
                let idx = (minute - min_m) as usize;
                if idx < len {
                    vec[idx] = count;
                }
            }
            vec
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::{drain3::Config, pipeline::ClpDrainPipeline};

    fn observe(pipe: &mut ClpDrainPipeline, store: &mut PatternStore, line: &str) {
        let parsed = pipe.process_line(line);
        store.accumulate(
            parsed.pattern_id,
            &parsed.display_template,
            &parsed.variables,
            &parsed.embedded_tokens,
            None,
            line,
            store.global_line_count + 1,
        );
    }

    #[test]
    fn test_late_widening_reconciles_all_slot_counts_and_numeric_stats() {
        let mut pipe = ClpDrainPipeline::new(Config::default());
        let mut store = PatternStore::new(0);
        let n = 10_000;
        for value in 1..=n {
            observe(
                &mut pipe,
                &mut store,
                &format!(
                    "INFO route=/foo/1/2 action accepted elapsed={value} bytes={} end",
                    value * 2,
                ),
            );
        }
        // Also exercise finalization before further accumulation and widening.
        store.finalize();
        observe(
            &mut pipe,
            &mut store,
            &format!(
                "INFO route=/bar/3/4 action rejected elapsed={} bytes={} tail",
                n + 1,
                (n + 1) * 2,
            ),
        );
        store.finalize();
        store.finalize();
        assert_eq!(store.patterns.len(), 1);
        let stats = &store.patterns[&1];
        assert_eq!(
            stats.template,
            "INFO <*> action <*> elapsed=<*> bytes=<*> <*>"
        );
        assert_eq!(stats.count, n + 1);
        assert_eq!(stats.variables.len(), 5);
        for (index, slot) in stats.variables.iter().enumerate() {
            assert_eq!(slot.slot_index, index);
            assert_eq!(slot.categorical.total_count, n + 1);
        }
        for (index, old, new) in [
            (0, "route=/foo/1/2", "route=/bar/3/4"),
            (1, "accepted", "rejected"),
            (4, "end", "tail"),
        ] {
            let slot = &stats.variables[index];
            assert!(slot.numeric.is_none());
            assert_eq!(slot.categorical.exact_counts.len(), 2);
            assert_eq!(slot.categorical.exact_counts[old], n);
            assert_eq!(slot.categorical.exact_counts[new], 1);
        }
        for (index, multiplier) in [(2, 1), (3, 2)] {
            let numeric = stats.variables[index].numeric.as_ref().unwrap();
            assert_eq!(numeric.count, n + 1);
            assert_eq!(numeric.min, multiplier as f64);
            assert_eq!(numeric.max, ((n + 1) * multiplier) as f64);
            assert_eq!(numeric.sum, ((n + 1) * (n + 2) / 2 * multiplier) as f64);
            assert_eq!(numeric.mean(), ((n + 2) * multiplier) as f64 / 2.0);
            let median = ((n / 2 + 1) * multiplier) as f64;
            assert!((numeric.quantile(0.5).unwrap() / median - 1.0).abs() < 0.011);
        }
    }

    #[test]
    fn test_capped_embedded_history_keeps_other_numeric_slots_exact() {
        let mut pipe = ClpDrainPipeline::new(Config::default());
        let mut store = PatternStore::new(0);
        let n = 2_000;
        for value in 1..=n {
            observe(
                &mut pipe,
                &mut store,
                &format!(
                    "INFO request path=/foo/{value}/{} elapsed={value}",
                    value + 1,
                ),
            );
        }
        let history = store.patterns[&1].embedded_history[2].as_ref().unwrap();
        assert_eq!(history.categorical.exact_counts.len(), EMBEDDED_HISTORY_CAP);
        assert_eq!(history.categorical.total_count, n);
        assert!(history.categorical.capped);
        observe(
            &mut pipe,
            &mut store,
            &format!("INFO request path=/bar/9/10 elapsed={}", n + 1,),
        );
        store.finalize();
        assert_eq!(store.patterns.len(), 1);
        let stats = &store.patterns[&1];
        assert_eq!(stats.template, "INFO request <*> elapsed=<*>");
        assert_eq!(stats.variables.len(), 2);
        assert!(stats.embedded_history[2].is_none());
        let path = &stats.variables[0];
        assert!(path.numeric.is_none());
        assert_eq!(path.categorical.total_count, n + 1);
        assert_eq!(path.categorical.exact_counts["path=/foo/1/2"], 1);
        // The cap deliberately omits later values, but never changes totals.
        assert!(
            !path
                .categorical
                .exact_counts
                .contains_key("path=/foo/100/101")
        );
        let elapsed = stats.variables[1].numeric.as_ref().unwrap();
        assert_eq!(stats.variables[1].categorical.total_count, n + 1);
        assert_eq!(elapsed.count, n + 1);
        assert_eq!(elapsed.sum, ((n + 1) * (n + 2) / 2) as f64);
        assert_eq!(elapsed.mean(), (n + 2) as f64 / 2.0);
        assert_eq!((elapsed.min, elapsed.max), (1.0, (n + 1) as f64));
    }

    #[test]
    fn test_plain_numeric_widening_backfills_a_billion_observations() {
        // Exercise the plain-numeric contract directly: normal CLP encoding
        // usually turns a numeric token into a placeholder before Drain sees it.
        let mut stats = PatternStats::new(1, "INFO job 7".into(), 0);
        stats.count = 1_000_000_000;
        stats.reconcile_template("INFO job <*>");
        let slot = &stats.variables[0];
        assert_eq!(slot.slot_index, 0);
        assert_eq!(slot.var_type, VarType::Integer);
        assert_eq!(slot.categorical.total_count, stats.count);
        assert_eq!(slot.categorical.exact_counts["7"], stats.count);
        let numeric = slot.numeric.as_ref().unwrap();
        assert_eq!(numeric.count, stats.count);
        assert_eq!(numeric.sketch.count(), stats.count as usize);
        assert_eq!(numeric.sum, 7_000_000_000.0);
        assert_eq!((numeric.min, numeric.max, numeric.mean()), (7.0, 7.0, 7.0));
        for q in [0.0, 0.5, 0.99, 1.0] {
            assert!((numeric.quantile(q).unwrap() / 7.0 - 1.0).abs() < 0.011);
        }
    }
}
