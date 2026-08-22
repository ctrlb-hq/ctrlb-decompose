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

    pub fn merge(&mut self, other: BoundedVec<T>) {
        if self.capacity == 0 {
            self.total_seen += other.total_seen;
            return;
        }
        let other_items_len = other.items.len();
        for item in other.items {
            self.push(item);
        }
        self.total_seen = self.total_seen.max(self.items.len() as u64)
            + other.total_seen.saturating_sub(other_items_len as u64);
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

    pub fn merge(&mut self, other: NumericStats) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = other;
            return;
        }
        self.count += other.count;
        self.sum += other.sum;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        let _ = self.sketch.merge(&other.sketch);
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

    pub fn merge(&mut self, other: CategoricalStats) {
        self.total_count += other.total_count;

        if self.capped || other.capped {
            self.capped = true;
            let mut hll = self.hll.take().unwrap_or_else(|| {
                let mut h =
                    HyperLogLogPlus::new(16, RandomState::new()).expect("HLL creation failed");
                for key in self.exact_counts.keys() {
                    h.insert(key);
                }
                h
            });

            if let Some(other_hll) = other.hll {
                hll.merge(&other_hll).ok();
            } else {
                for key in other.exact_counts.keys() {
                    hll.insert(key);
                }
            }
            self.hll = Some(hll);

            for (k, v) in other.exact_counts {
                if let Some(cnt) = self.exact_counts.get_mut(&k) {
                    *cnt += v;
                } else if self.exact_counts.len() < CARDINALITY_CAP {
                    self.exact_counts.insert(k, v);
                }
            }
        } else {
            for (k, v) in other.exact_counts {
                *self.exact_counts.entry(k).or_insert(0) += v;
            }
            if self.exact_counts.len() > CARDINALITY_CAP {
                self.capped = true;
                let mut hll =
                    HyperLogLogPlus::new(16, RandomState::new()).expect("HLL creation failed");
                for key in self.exact_counts.keys() {
                    hll.insert(key);
                }
                self.hll = Some(hll);
            } else {
                self.cached_unique = self.exact_counts.len() as u64;
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

    pub fn merge(&mut self, other: VarSlotStats) {
        for (i, &votes) in other.type_votes.iter().enumerate() {
            self.type_votes[i] += votes;
        }
        self.categorical.merge(other.categorical);
        if let Some(other_num) = other.numeric {
            if let Some(ref mut num) = self.numeric {
                num.merge(other_num);
            } else {
                self.numeric = Some(other_num);
            }
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

    pub fn merge(&mut self, other: PatternStats) {
        self.count += other.count;
        if self.first_seen_line == 0
            || (other.first_seen_line > 0 && other.first_seen_line < self.first_seen_line)
        {
            self.first_seen_line = other.first_seen_line;
        }
        self.last_seen_line = self.last_seen_line.max(other.last_seen_line);

        match (self.first_ts, other.first_ts) {
            (None, Some(ts)) => self.first_ts = Some(ts),
            (Some(t1), Some(t2)) if t2 < t1 => self.first_ts = Some(t2),
            _ => {}
        }
        match (self.last_ts, other.last_ts) {
            (None, Some(ts)) => self.last_ts = Some(ts),
            (Some(t1), Some(t2)) if t2 > t1 => self.last_ts = Some(t2),
            _ => {}
        }

        for (minute, count) in other.time_buckets {
            *self.time_buckets.entry(minute).or_insert(0) += count;
        }

        while self.variables.len() < other.variables.len() {
            self.variables.push(VarSlotStats::new(self.variables.len()));
        }
        for (i, var) in other.variables.into_iter().enumerate() {
            self.variables[i].merge(var);
        }

        self.example_lines.merge(other.example_lines);
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

    pub fn merge(&mut self, other: PatternStore) {
        self.global_line_count += other.global_line_count;
        match (self.global_first_ts, other.global_first_ts) {
            (None, Some(ts)) => self.global_first_ts = Some(ts),
            (Some(t1), Some(t2)) if t2 < t1 => self.global_first_ts = Some(t2),
            _ => {}
        }
        match (self.global_last_ts, other.global_last_ts) {
            (None, Some(ts)) => self.global_last_ts = Some(ts),
            (Some(t1), Some(t2)) if t2 > t1 => self.global_last_ts = Some(t2),
            _ => {}
        }

        let mut template_to_id: HashMap<String, PatternID> = self
            .patterns
            .iter()
            .map(|(&id, s)| (s.template.clone(), id))
            .collect();

        for (_, other_stats) in other.patterns {
            if let Some(&existing_id) = template_to_id.get(&other_stats.template) {
                self.patterns
                    .get_mut(&existing_id)
                    .unwrap()
                    .merge(other_stats);
            } else {
                let next_id = self.patterns.keys().max().copied().unwrap_or(0) + 1;
                template_to_id.insert(other_stats.template.clone(), next_id);
                let mut new_stats = other_stats;
                new_stats.pattern_id = next_id;
                self.patterns.insert(next_id, new_stats);
            }
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

    #[test]
    fn test_pattern_store_merge_equivalence() {
        let mut single_store = PatternStore::new(2);
        let mut store_a = PatternStore::new(2);
        let mut store_b = PatternStore::new(2);

        let var1 = TypedVariable {
            raw: "42".to_string(),
            var_type: VarType::Integer,
        };
        let var2 = TypedVariable {
            raw: "100".to_string(),
            var_type: VarType::Integer,
        };

        // Single store processes 2 items
        single_store.accumulate(
            1,
            "Request id <*>",
            std::slice::from_ref(&var1),
            None,
            "Request id 42",
            1,
        );
        single_store.accumulate(
            1,
            "Request id <*>",
            std::slice::from_ref(&var2),
            None,
            "Request id 100",
            2,
        );

        // Store A processes item 1, Store B processes item 2
        store_a.accumulate(
            1,
            "Request id <*>",
            std::slice::from_ref(&var1),
            None,
            "Request id 42",
            1,
        );
        store_b.accumulate(
            2, // different local ID
            "Request id <*>",
            std::slice::from_ref(&var2),
            None,
            "Request id 100",
            2,
        );

        // Merge store B into store A
        store_a.merge(store_b);

        single_store.finalize();
        store_a.finalize();

        assert_eq!(single_store.global_line_count, store_a.global_line_count);
        assert_eq!(single_store.patterns.len(), store_a.patterns.len());

        let single_pat = single_store.patterns.values().next().unwrap();
        let merged_pat = store_a.patterns.values().next().unwrap();

        assert_eq!(single_pat.count, merged_pat.count);
        assert_eq!(single_pat.template, merged_pat.template);
        assert_eq!(
            single_pat.variables[0].var_type,
            merged_pat.variables[0].var_type
        );
        assert_eq!(
            single_pat.variables[0].numeric.as_ref().unwrap().count,
            merged_pat.variables[0].numeric.as_ref().unwrap().count
        );
        assert_eq!(
            single_pat.variables[0].numeric.as_ref().unwrap().sum,
            merged_pat.variables[0].numeric.as_ref().unwrap().sum
        );
    }
}
