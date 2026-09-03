use crate::extraction::clp::core::{
    EightByteEncodedVariable, EncodedVariable, VariablePlaceholder, encode_float_string,
    encode_integer_string, escape_and_append_const_to_logtype, get_bounds_of_next_var,
};

#[derive(Debug, Clone)]
pub struct EncodingStats {
    pub total_processed: usize,
    pub logtype_buffer_capacity: usize,
    pub encoded_vars_capacity: usize,
    pub dictionary_vars_capacity: usize,
}

/// A reusable context for encoding CLP log messages
pub struct EncodingContext<T: EncodedVariable> {
    logtype: String,
    encoded_vars: Vec<T>,
    dictionary_vars: Vec<String>,
    // Statistics for monitoring
    total_processed: usize,
}

impl<T: EncodedVariable> EncodingContext<T> {
    /// Create a new context with pre-allocated capacity
    pub fn new(estimated_logtype_size: usize, estimated_vars_count: usize) -> Self {
        Self {
            logtype: String::with_capacity(estimated_logtype_size),
            encoded_vars: Vec::with_capacity(estimated_vars_count),
            dictionary_vars: Vec::with_capacity(estimated_vars_count),
            total_processed: 0,
        }
    }

    /// Reset the context for reuse without deallocating memory
    pub fn clear(&mut self) {
        self.logtype.clear();
        self.encoded_vars.clear();
        self.dictionary_vars.clear();
        // Keep statistics
        self.total_processed += 1;
    }

    /// Encode a message using this context
    pub fn encode_message(&mut self, message: &str) -> (&str, &[T], &[String]) {
        // Clear previous data
        self.clear();

        // Use the existing implementation but store results in self fields
        encode_message_into(
            message,
            &mut self.logtype,
            &mut self.encoded_vars,
            &mut self.dictionary_vars,
        );

        // Return references to the internal data
        (&self.logtype, &self.encoded_vars, &self.dictionary_vars)
    }

    /// Manually resize buffers if needed for better performance
    pub fn resize_buffers(&mut self, logtype_capacity: usize, vars_capacity: usize) {
        self.logtype.reserve(logtype_capacity);
        self.encoded_vars.reserve(vars_capacity);
        self.dictionary_vars.reserve(vars_capacity);
    }

    /// Get statistics about context usage
    pub fn stats(&self) -> EncodingStats {
        EncodingStats {
            total_processed: self.total_processed,
            logtype_buffer_capacity: self.logtype.capacity(),
            encoded_vars_capacity: self.encoded_vars.capacity(),
            dictionary_vars_capacity: self.dictionary_vars.capacity(),
        }
    }
}

/// Modified version that uses pre-allocated buffers
pub fn encode_message_into<T: EncodedVariable>(
    message: &str,
    logtype: &mut String,
    encoded_vars: &mut Vec<T>,
    dictionary_vars: &mut Vec<String>,
) {
    let mut var_begin_pos = 0;
    let mut var_end_pos = 0;

    while let Some((begin_pos, end_pos)) =
        get_bounds_of_next_var(message, var_begin_pos, var_end_pos)
    {
        // Process constant text before this variable
        let constant = &message[var_begin_pos..begin_pos];
        escape_and_append_const_to_logtype(constant, logtype);

        // Update positions
        var_begin_pos = begin_pos;
        var_end_pos = end_pos;

        // Process the variable
        let var_string = &message[var_begin_pos..var_end_pos];

        if let Some(encoded_var) = encode_float_string::<T>(var_string) {
            // Float variable
            logtype.push(VariablePlaceholder::Float as u8 as char);
            encoded_vars.push(encoded_var);
        } else if let Some(encoded_var) = encode_integer_string::<T>(var_string) {
            // Integer variable
            logtype.push(VariablePlaceholder::Integer as u8 as char);
            encoded_vars.push(encoded_var);
        } else {
            // Dictionary variable - store the actual string value
            logtype.push(VariablePlaceholder::Dictionary as u8 as char);
            dictionary_vars.push(var_string.to_string());
        }

        // Move to position after this variable
        var_begin_pos = var_end_pos;
    }

    // Process any remaining constant text
    if var_begin_pos < message.len() {
        let constant = &message[var_begin_pos..];
        escape_and_append_const_to_logtype(constant, logtype);
    }
}

/// Thread-local encoding context for better performance in single-threaded scenarios
thread_local! {
    static THREAD_LOCAL_ENCODE_CONTEXT: std::cell::RefCell<EncodingContext<EightByteEncodedVariable>> =
        std::cell::RefCell::new(EncodingContext::new(2048, 128));
}

/// High-performance encode function using thread-local context
pub fn encode_message_fast(message: &str) -> (String, Vec<EightByteEncodedVariable>, Vec<String>) {
    THREAD_LOCAL_ENCODE_CONTEXT.with(|context| {
        let mut ctx = context.borrow_mut();
        let (logtype, encoded_vars, dictionary_vars) = ctx.encode_message(message);
        (
            logtype.to_string(),
            encoded_vars.to_vec(),
            dictionary_vars.to_vec(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::clp::core::EightByteEncodedVariable;

    #[test]
    fn test_encoding_context_basic() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(1024, 64);

        let message = "User ID=123 logged in from 192.168.1.1 with balance -45.67";
        {
            let (logtype, encoded_vars, dictionary_vars) = context.encode_message(message);

            println!("Logtype: {:?}", logtype);
            println!("Encoded vars: {:?}", encoded_vars);
            println!("Dictionary vars: {:?}", dictionary_vars);

            assert!(!logtype.is_empty());
            assert!(!encoded_vars.is_empty());
            assert!(!dictionary_vars.is_empty());
        }

        println!("Stats: {:?}", context.stats());
    }

    #[test]
    fn test_context_reuse() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(1024, 64);

        // Process multiple messages to test reuse
        for i in 0..100 {
            let message = format!("Message {}: Processing item {}", i, i * 2);
            let (logtype, encoded_vars, dictionary_vars) = context.encode_message(&message);

            assert!(!logtype.is_empty());
            // Verify that we have some variables
            assert!(encoded_vars.len() + dictionary_vars.len() > 0);
        }

        let stats = context.stats();
        assert_eq!(stats.total_processed, 100);
        println!("Final stats after 100 encodes: {:?}", stats);
    }

    #[test]
    fn test_different_variable_types() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(512, 32);

        // Test message with different variable types
        let message =
            "User john.doe processed payment $123.45 at timestamp 1640995200 with session abc123";
        let (logtype, encoded_vars, dictionary_vars) = context.encode_message(message);

        println!("Message: {}", message);
        println!("Logtype: {:?}", logtype);
        println!("Encoded vars count: {}", encoded_vars.len());
        println!("Dictionary vars: {:?}", dictionary_vars);

        // Should have encoded variables (numbers: 123.45, 1640995200)
        // and dictionary variables (abc123 — contains digits + alpha)
        assert!(encoded_vars.len() > 0);
        assert!(dictionary_vars.contains(&"abc123".to_string()));
    }

    #[test]
    fn test_empty_and_edge_cases() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(256, 16);

        // Test empty message
        let (logtype, encoded_vars, dictionary_vars) = context.encode_message("");
        assert!(logtype.is_empty());
        assert!(encoded_vars.is_empty());
        assert!(dictionary_vars.is_empty());

        // Test message with only constants
        let (logtype, encoded_vars, dictionary_vars) = context.encode_message("Hello World");
        assert_eq!(logtype, "Hello World");
        assert!(encoded_vars.is_empty());
        assert!(dictionary_vars.is_empty());

        // Test message with only variables
        let (logtype, encoded_vars, dictionary_vars) = context.encode_message("123");
        assert!(!logtype.is_empty());
        assert!(encoded_vars.len() + dictionary_vars.len() > 0);
    }

    #[test]
    fn test_buffer_growth() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(10, 2); // Very small initial capacity

        let initial_stats = context.stats();
        println!(
            "Initial capacities: logtype={}, encoded={}, dict={}",
            initial_stats.logtype_buffer_capacity,
            initial_stats.encoded_vars_capacity,
            initial_stats.dictionary_vars_capacity
        );

        // Process a large message that will cause buffer growth
        let large_message = "This is a very long message with many variables like ID=12345 and name=john.doe and email=test@example.com and timestamp=1640995200 and amount=999.99 and session=very_long_session_id_12345 that should cause buffer growth";

        {
            let (logtype, encoded_vars, dictionary_vars) = context.encode_message(large_message);

            assert!(!logtype.is_empty());
            assert!(encoded_vars.len() > 0);
            assert!(dictionary_vars.len() > 0);
        }

        let final_stats = context.stats();
        println!(
            "Final capacities: logtype={}, encoded={}, dict={}",
            final_stats.logtype_buffer_capacity,
            final_stats.encoded_vars_capacity,
            final_stats.dictionary_vars_capacity
        );

        assert!(final_stats.logtype_buffer_capacity >= initial_stats.logtype_buffer_capacity);
        assert!(final_stats.encoded_vars_capacity >= initial_stats.encoded_vars_capacity);
        assert!(final_stats.dictionary_vars_capacity >= initial_stats.dictionary_vars_capacity);
    }

    #[test]
    fn test_thread_local_fast_encoding() {
        // Test the fast encoding function
        let message = "Request ID=12345 processed in 250ms by user admin";
        let (logtype, encoded_vars, dictionary_vars) = encode_message_fast(message);

        assert!(!logtype.is_empty());
        assert!(encoded_vars.len() > 0);
        assert!(dictionary_vars.len() > 0);

        println!("Fast encoding result:");
        println!("  Logtype: {:?}", logtype);
        println!("  Encoded vars: {:?}", encoded_vars);
        println!("  Dictionary vars: {:?}", dictionary_vars);
    }

    #[test]
    fn benchmark_encoding_performance() {
        use std::time::Instant;

        let test_messages = vec![
            "User ID=12345 logged in from 192.168.1.100 at 2023-10-15T14:30:25Z",
            "Error: Connection timeout after 30.5 seconds for user session abc123",
            "Payment of $125.99 processed successfully for order #ORD-789456",
            "API request /api/v1/users/profile took 45ms to complete",
            "Warning: Memory usage at 85.7% of 8GB limit",
        ];

        // Benchmark with context reuse
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(2048, 128);
        let start = Instant::now();
        for _ in 0..1000 {
            for message in &test_messages {
                let _result = context.encode_message(message);
            }
        }
        let context_time = start.elapsed();

        // Benchmark without context (allocating each time)
        let start = Instant::now();
        for _ in 0..1000 {
            for message in &test_messages {
                // Simulate what happens without context reuse
                let mut logtype = String::new();
                let mut encoded_vars: Vec<EightByteEncodedVariable> = Vec::new();
                let mut dictionary_vars = Vec::new();
                encode_message_into(
                    message,
                    &mut logtype,
                    &mut encoded_vars,
                    &mut dictionary_vars,
                );
            }
        }
        let no_context_time = start.elapsed();

        // Benchmark with thread-local context
        let start = Instant::now();
        for _ in 0..1000 {
            for message in &test_messages {
                let _result = encode_message_fast(message);
            }
        }
        let thread_local_time = start.elapsed();

        println!("Encoding Performance Comparison (5000 messages):");
        println!("  With context reuse: {:?}", context_time);
        println!("  Without context: {:?}", no_context_time);
        println!("  Thread-local context: {:?}", thread_local_time);

        if no_context_time.as_nanos() > 0 {
            println!(
                "  Context speedup: {:.2}x",
                no_context_time.as_nanos() as f64 / context_time.as_nanos() as f64
            );
            println!(
                "  Thread-local speedup: {:.2}x",
                no_context_time.as_nanos() as f64 / thread_local_time.as_nanos() as f64
            );
        }

        let final_stats = context.stats();
        println!("Final context stats: {:?}", final_stats);
    }

    #[test]
    fn test_manual_buffer_resize() {
        let mut context = EncodingContext::<EightByteEncodedVariable>::new(100, 10);

        let initial_stats = context.stats();
        println!("Before resize: {:?}", initial_stats);

        // Manually resize buffers
        context.resize_buffers(2000, 200);

        let after_resize_stats = context.stats();
        println!("After resize: {:?}", after_resize_stats);

        // Capacities should be at least what we requested
        assert!(after_resize_stats.logtype_buffer_capacity >= 2000);
        assert!(after_resize_stats.encoded_vars_capacity >= 200);
        assert!(after_resize_stats.dictionary_vars_capacity >= 200);

        // Test that it still works after resize
        let message = "Test message ID=123 with session abc456";
        let (logtype, encoded_vars, dictionary_vars) = context.encode_message(message);

        assert!(!logtype.is_empty());
        assert!(encoded_vars.len() > 0);
        assert!(dictionary_vars.contains(&"abc456".to_string()));
    }
}
