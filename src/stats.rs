use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hyperloglogplus::{HyperLogLog, HyperLogLogPlus};
use sketches_ddsketch::{Config as DDSketchConfig, DDSketch};
use std::collections::hash_map::RandomState;

use crate::extraction::drain3::TypedVariable;
use crate::types::{PatternID, VarType};

const CARDINALITY_CAP: usize = 10_000;

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

impl Default for NumericStats {
    fn default() -> Self {
        Self::new()
    }
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
        self.sketch.add(value);
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
    hll: Option<HyperLogLogPlus<String, RandomState>>,
    cached_unique: u64,
    capped: bool,
}

impl Default for CategoricalStats {
    fn default() -> Self {
        Self::new()
    }
}

impl CategoricalStats {
    pub fn new() -> Self {
        CategoricalStats {
            total_count: 0,
            exact_counts: HashMap::new(),
            hll: None,
            cached_unique: 0,
            capped: false,
        }
    }

    pub fn update(&mut self, value: &str) {
        self.total_count += 1;

        if self.capped {
            // Only increment existing entries; use HLL for cardinality
            if let Some(count) = self.exact_counts.get_mut(value) {
                *count += 1;
            }
            if let Some(ref mut hll) = self.hll {
                hll.insert(&value.to_string());
            }
        } else {
            *self.exact_counts.entry(value.to_string()).or_insert(0) += 1;
            self.cached_unique = self.exact_counts.len() as u64;
            if self.exact_counts.len() > CARDINALITY_CAP {
                self.capped = true;
                let mut hll =
                    HyperLogLogPlus::new(16, RandomState::new()).expect("HLL creation failed");
                for key in self.exact_counts.keys() {
                    hll.insert(key);
                }
                self.hll = Some(hll);
            }
        }
    }

    pub fn finalize(&mut self) {
        if self.capped
            && let Some(ref mut hll) = self.hll
        {
            self.cached_unique = hll.count().round() as u64;
        }
    }

    pub fn unique_count(&self) -> u64 {
        self.cached_unique
    }

    /// Returns top-k entries as (value, count, percentage)
    pub fn top_k(&self, k: usize) -> Vec<(String, u64, f64)> {
        let mut entries: Vec<_> = self
            .exact_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        entries.sort_by_key(|b| std::cmp::Reverse(b.1));
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
    type_votes: [u64; 10],
}

impl VarSlotStats {
    pub fn new(slot_index: usize) -> Self {
        VarSlotStats {
            slot_index,
            var_type: VarType::String,
            numeric: None,
            categorical: CategoricalStats::new(),
            type_votes: [0; 10],
        }
    }

    pub fn update(&mut self, var: &TypedVariable) {
        // Fast vote recording into fixed array
        self.type_votes[var.var_type as usize] += 1;

        // Always update categorical
        self.categorical.update(&var.raw);

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

    pub fn finalize(&mut self) {
        // Resolve type vote (majority wins, deterministic tie-breaking on enum order)
        let mut max_votes = 0;
        let mut best_type = self.var_type;
        for &vt in &VarType::ALL {
            let votes = self.type_votes[vt as usize];
            if votes > max_votes || (votes == max_votes && max_votes > 0 && vt < best_type) {
                max_votes = votes;
                best_type = vt;
            }
        }
        self.var_type = best_type;

        self.categorical.finalize();
        self.check_enum_reclassify();
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
        if let Some(num_str) = raw.strip_suffix(suffix)
            && let Ok(v) = num_str.parse::<f64>()
        {
            return Some(v * multiplier);
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
}

impl PatternStats {
    pub fn new(pattern_id: PatternID, template: String, context_lines: usize) -> Self {
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
        }
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

        // Update template only when it changes from Drain3
        if stats.template != template {
            stats.template = template.to_string();
        }

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

        // Example lines (only allocate string when context lines > 0)
        if self.context_lines > 0 {
            stats.example_lines.push(raw_line.to_string());
        }
    }

    /// Run post-accumulation fixups (finalize categorical HLL, enum reclassification, etc.)
    pub fn finalize(&mut self) {
        for stats in self.patterns.values_mut() {
            for var in &mut stats.variables {
                var.finalize();
            }
        }
    }

    /// Patterns sorted by count descending
    pub fn sorted_patterns(&self) -> Vec<&PatternStats> {
        let mut patterns: Vec<_> = self.patterns.values().collect();
        patterns.sort_by_key(|a| std::cmp::Reverse(a.count));
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

    #[test]
    fn test_categorical_stats_high_cardinality_deferred_hll() {
        let mut stats = CategoricalStats::new();

        // Ingest 30,000 unique values (exceeds CARDINALITY_CAP of 10,000)
        let total_unique = 30_000;
        for i in 0..total_unique {
            stats.update(&format!("val_{:06}", i));
        }

        assert_eq!(stats.total_count, total_unique as u64);
        assert!(stats.capped);

        // Before finalize, cached_unique represents the cap snapshot
        // Finalize computes the true HLL estimate
        stats.finalize();

        let unique = stats.unique_count();
        // HLL with p=16 has standard error ~1.04 / sqrt(2^16) ≈ 0.4%
        let error_margin = (total_unique as f64 * 0.05) as u64;
        assert!(
            unique >= total_unique as u64 - error_margin
                && unique <= total_unique as u64 + error_margin,
            "Expected ~{}, got {}",
            total_unique,
            unique
        );
    }

    #[test]
    fn test_deterministic_type_voting_on_tie() {
        let mut slot1 = VarSlotStats::new(0);
        let mut slot2 = VarSlotStats::new(0);

        // Cast 1 vote for Integer, 1 vote for Float
        slot1.update(&TypedVariable {
            raw: "100".to_string(),
            var_type: VarType::Integer,
        });
        slot1.update(&TypedVariable {
            raw: "100.5".to_string(),
            var_type: VarType::Float,
        });

        // Inverse order in slot 2
        slot2.update(&TypedVariable {
            raw: "100.5".to_string(),
            var_type: VarType::Float,
        });
        slot2.update(&TypedVariable {
            raw: "100".to_string(),
            var_type: VarType::Integer,
        });

        slot1.finalize();
        slot2.finalize();

        assert_eq!(slot1.var_type, slot2.var_type);
        assert_eq!(slot1.var_type, VarType::Integer);
    }

    #[test]
    fn test_numeric_stats_quantiles() {
        let mut num = NumericStats::new();
        for i in 1..=100 {
            num.update(i as f64);
        }

        assert_eq!(num.count, 100);
        assert_eq!(num.min, 1.0);
        assert_eq!(num.max, 100.0);
        assert_eq!(num.mean(), 50.5);

        let p50 = num.quantile(0.50).unwrap();
        assert!((p50 - 50.0).abs() <= 2.0);
    }

    #[test]
    fn test_bounded_vec_reservoir_sampling() {
        let mut bounded = BoundedVec::new(5);
        for i in 0..100 {
            bounded.push(i);
        }

        assert_eq!(bounded.items().len(), 5);
        assert_eq!(bounded.total_seen, 100);
    }

    #[test]
    fn test_var_slot_stats_finalize_type_voting() {
        let mut slot = VarSlotStats::new(0);
        // Cast 2 votes for Integer, 1 for String
        slot.update(&TypedVariable {
            raw: "100".to_string(),
            var_type: VarType::Integer,
        });
        slot.update(&TypedVariable {
            raw: "200".to_string(),
            var_type: VarType::Integer,
        });
        slot.update(&TypedVariable {
            raw: "text".to_string(),
            var_type: VarType::String,
        });

        slot.finalize();
        assert_eq!(slot.var_type, VarType::Integer);
    }

    #[test]
    fn test_bounded_vec_zero_capacity() {
        let mut bounded = BoundedVec::new(0);
        bounded.push("line1".to_string());
        bounded.push("line2".to_string());
        assert_eq!(bounded.items().len(), 0);
        assert_eq!(bounded.total_seen, 2);
    }
}
