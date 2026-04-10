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

    pub fn total_seen(&self) -> u64 {
        self.total_seen
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
                self.cached_unique = hll.count().round() as u64;
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
                self.cached_unique = hll.count().round() as u64;
                self.hll = Some(hll);
            }
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
        // Vote on type (majority wins)
        *self.type_votes.entry(var.var_type).or_insert(0) += 1;
        self.var_type = *self.type_votes.iter().max_by_key(|(_, v)| *v).unwrap().0;

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

        // Always update template to the latest from Drain3 — it evolves
        // as more lines are seen (more positions become <*>)
        stats.template = template.to_string();

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
                stats.variables.push(VarSlotStats::new(stats.variables.len()));
            }
            stats.variables[i].update(var);
        }

        // Example lines
        stats.example_lines.push(raw_line.to_string());
    }

    /// Run post-accumulation fixups (enum reclassification, etc.)
    pub fn finalize(&mut self) {
        for stats in self.patterns.values_mut() {
            for var in &mut stats.variables {
                var.check_enum_reclassify();
            }
        }
    }

    /// Patterns sorted by count descending, with pattern_id as a stable tie-breaker
    pub fn sorted_patterns(&self) -> Vec<&PatternStats> {
        let mut patterns: Vec<_> = self.patterns.values().collect();
        patterns.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.pattern_id.cmp(&b.pattern_id)));
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
    use crate::extraction::drain3::TypedVariable;
    use crate::types::VarType;

    // ---- BoundedVec tests ----

    #[test]
    fn test_bounded_vec_basic() {
        let mut bv = BoundedVec::new(3);
        bv.push(10);
        bv.push(20);
        bv.push(30);
        assert_eq!(bv.items(), &[10, 20, 30]);
        assert_eq!(bv.total_seen(), 3);
    }

    #[test]
    fn test_bounded_vec_zero_capacity() {
        let mut bv = BoundedVec::new(0);
        bv.push(1);
        bv.push(2);
        bv.push(3);
        assert!(bv.items().is_empty());
        assert_eq!(bv.total_seen(), 3);
    }

    #[test]
    fn test_bounded_vec_reservoir_sampling() {
        let mut bv = BoundedVec::new(10);
        for i in 0..1000 {
            bv.push(i);
        }
        assert_eq!(bv.items().len(), 10);
        assert_eq!(bv.total_seen(), 1000);
    }

    // ---- NumericStats tests ----

    #[test]
    fn test_numeric_stats_basic() {
        let mut ns = NumericStats::new();
        ns.update(10.0);
        ns.update(20.0);
        ns.update(30.0);
        assert_eq!(ns.count, 3);
        assert_eq!(ns.min, 10.0);
        assert_eq!(ns.max, 30.0);
        assert!((ns.mean() - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_numeric_stats_empty() {
        let ns = NumericStats::new();
        assert_eq!(ns.count, 0);
        assert!((ns.mean() - 0.0).abs() < f64::EPSILON);
        assert!(ns.quantile(0.5).is_none());
    }

    #[test]
    fn test_numeric_stats_nan_inf() {
        // Documenting current behavior:
        // - NaN is accepted (DDSketch silently handles it), count is incremented
        // - Inf and -Inf cause DDSketch to panic internally
        let mut ns = NumericStats::new();
        ns.update(1.0);
        ns.update(f64::NAN);
        assert_eq!(ns.count, 2, "NaN should be counted");

        let result = std::panic::catch_unwind(|| {
            let mut ns = NumericStats::new();
            ns.update(f64::INFINITY);
            ns
        });
        assert!(result.is_err(), "DDSketch should panic on Inf");

        let result = std::panic::catch_unwind(|| {
            let mut ns = NumericStats::new();
            ns.update(f64::NEG_INFINITY);
            ns
        });
        assert!(result.is_err(), "DDSketch should panic on -Inf");
    }

    #[test]
    fn test_numeric_stats_single_value() {
        let mut ns = NumericStats::new();
        ns.update(42.0);
        assert_eq!(ns.count, 1);
        assert_eq!(ns.min, 42.0);
        assert_eq!(ns.max, 42.0);
        assert!((ns.mean() - 42.0).abs() < f64::EPSILON);
        assert!((ns.sum - 42.0).abs() < f64::EPSILON);
    }

    // ---- CategoricalStats tests ----

    #[test]
    fn test_categorical_stats_basic() {
        let mut cs = CategoricalStats::new();
        cs.update("a");
        cs.update("a");
        cs.update("b");
        assert_eq!(cs.total_count, 3);
        assert_eq!(cs.unique_count(), 2);
        let top = cs.top_k(10);
        assert_eq!(top.len(), 2);
        // "a" should be first (count 2), "b" second (count 1)
        assert_eq!(top[0].0, "a");
        assert_eq!(top[0].1, 2);
        assert_eq!(top[1].0, "b");
        assert_eq!(top[1].1, 1);
    }

    #[test]
    fn test_categorical_stats_empty() {
        let cs = CategoricalStats::new();
        assert_eq!(cs.total_count, 0);
        assert_eq!(cs.unique_count(), 0);
        assert!(cs.top_k(10).is_empty());
    }

    #[test]
    fn test_categorical_stats_hll_transition() {
        let mut cs = CategoricalStats::new();
        let n = 12_000;
        for i in 0..n {
            cs.update(&format!("value_{}", i));
        }
        // After exceeding CARDINALITY_CAP (10_000), HLL should be active
        let unique = cs.unique_count();
        let expected = n as f64;
        let error = (unique as f64 - expected).abs() / expected;
        assert!(
            error < 0.10,
            "HLL estimate {} too far from expected {} (error: {:.2}%)",
            unique,
            n,
            error * 100.0
        );
    }

    // ---- parse_numeric_value tests ----

    #[test]
    fn test_parse_numeric_value_plain() {
        assert!((parse_numeric_value("42").unwrap() - 42.0).abs() < f64::EPSILON);
        assert!((parse_numeric_value("-3.14").unwrap() - (-3.14)).abs() < f64::EPSILON);
        assert!((parse_numeric_value("0").unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_numeric_value_durations() {
        assert!((parse_numeric_value("100ms").unwrap() - 100.0).abs() < f64::EPSILON);
        assert!((parse_numeric_value("1s").unwrap() - 1000.0).abs() < f64::EPSILON);
        assert!((parse_numeric_value("5m").unwrap() - 300_000.0).abs() < f64::EPSILON);
        assert!((parse_numeric_value("2h").unwrap() - 7_200_000.0).abs() < f64::EPSILON);
        assert!((parse_numeric_value("500us").unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((parse_numeric_value("500µs").unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((parse_numeric_value("1000ns").unwrap() - 0.001).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_numeric_value_invalid() {
        assert!(parse_numeric_value("hello").is_none());
        assert!(parse_numeric_value("").is_none());
        assert!(parse_numeric_value("abc123").is_none());
        assert!(parse_numeric_value("ms").is_none());
    }

    #[test]
    fn test_parse_numeric_value_special_floats() {
        // Documenting current behavior: Rust's f64::parse accepts these
        assert!(parse_numeric_value("inf").is_some());
        assert!(parse_numeric_value("-inf").is_some());
        assert!(parse_numeric_value("NaN").is_some());
    }

    // ---- VarSlotStats tests ----

    #[test]
    fn test_var_slot_stats_enum_reclassify() {
        let mut slot = VarSlotStats::new(0);

        // Feed 60 occurrences of 3 String-typed values (30+20+10)
        for _ in 0..30 {
            slot.update(&TypedVariable {
                raw: "alpha".to_string(),
                var_type: VarType::String,
            });
        }
        for _ in 0..20 {
            slot.update(&TypedVariable {
                raw: "beta".to_string(),
                var_type: VarType::String,
            });
        }
        for _ in 0..10 {
            slot.update(&TypedVariable {
                raw: "gamma".to_string(),
                var_type: VarType::String,
            });
        }

        // Before reclassify, type should be String (majority vote)
        assert_eq!(slot.var_type, VarType::String);

        // After reclassify, should become Enum (60 occurrences, 3 unique, top 3 cover 100%)
        slot.check_enum_reclassify();
        assert_eq!(slot.var_type, VarType::Enum);
    }

    // ---- PatternStore tests ----

    #[test]
    fn test_pattern_store_accumulate() {
        let mut store = PatternStore::new(5);

        let vars1 = vec![TypedVariable {
            raw: "200".to_string(),
            var_type: VarType::Integer,
        }];
        let vars2 = vec![TypedVariable {
            raw: "404".to_string(),
            var_type: VarType::Integer,
        }];

        store.accumulate(1, "GET <*> HTTP/1.1", &vars1, None, "GET /foo HTTP/1.1", 1);
        store.accumulate(1, "GET <*> HTTP/1.1", &vars2, None, "GET /bar HTTP/1.1", 2);

        assert_eq!(store.global_line_count, 2);
        assert_eq!(store.patterns.len(), 1);

        let pattern = store.patterns.get(&1).unwrap();
        assert_eq!(pattern.count, 2);
        assert_eq!(pattern.first_seen_line, 1);
        assert_eq!(pattern.last_seen_line, 2);
        assert_eq!(pattern.variables.len(), 1);
    }
}
