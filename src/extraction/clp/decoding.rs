use crate::extraction::clp::core::{
    EncodedVariable, VariablePlaceholder, decode_float_properties,
};

#[derive(Debug, Clone)]
pub struct DecodingStats {
    pub total_processed: usize,
    pub message_buffer_capacity: usize,
    pub temp_buffer_capacity: usize,
}

/// A reusable context for decoding CLP-encoded log messages
pub struct DecodingContext {
    message_buffer: String,
    temp_buffer: String,
    // Optional statistics for monitoring
    total_processed: usize,
}

impl DecodingContext {
    /// Create a new context with pre-allocated capacity
    pub fn new(estimated_message_size: usize, estimated_var_size: usize) -> Self {
        Self {
            message_buffer: String::with_capacity(estimated_message_size),
            temp_buffer: String::with_capacity(estimated_var_size),
            total_processed: 0,
        }
    }

    /// Reset the context for reuse without deallocating memory
    pub fn clear(&mut self) {
        self.message_buffer.clear();
        self.temp_buffer.clear();
        // Keep statistics
        self.total_processed += 1;
    }

    /// Decode a message using this context
    pub fn decode_message<T: EncodedVariable>(
        &mut self,
        logtype: &str,
        encoded_vars: &[T],
        dictionary_vars: &[String],
    ) -> &str {
        // Clear previous data
        self.clear();

        // Use the existing implementation but store results in self fields
        decode_message_into(
            logtype,
            encoded_vars,
            dictionary_vars,
            &mut self.message_buffer,
            &mut self.temp_buffer,
        );

        // Return reference to the internal data
        &self.message_buffer
    }

    /// Manually resize buffers if needed for better performance
    pub fn resize_buffers(&mut self, message_capacity: usize, temp_capacity: usize) {
        self.message_buffer.reserve(message_capacity);
        self.temp_buffer.reserve(temp_capacity);
    }

    /// Get statistics about context usage
    pub fn stats(&self) -> DecodingStats {
        DecodingStats {
            total_processed: self.total_processed,
            message_buffer_capacity: self.message_buffer.capacity(),
            temp_buffer_capacity: self.temp_buffer.capacity(),
        }
    }
}

/// Modified version that uses pre-allocated buffers
pub fn decode_message_into<T: EncodedVariable>(
    logtype: &str,
    encoded_vars: &[T],
    dictionary_vars: &[String],
    message_buffer: &mut String,
    temp_buffer: &mut String,
) {
    let mut encoded_var_index = 0;
    let mut dictionary_var_index = 0;

    // Process each character in the logtype
    let mut i = 0;
    while i < logtype.len() {
        let c = logtype.as_bytes()[i] as char;

        if c as u8 == VariablePlaceholder::Escape as u8 {
            // This is an escape character, so the next character should be taken literally
            i += 1;
            if i < logtype.len() {
                message_buffer.push(logtype.as_bytes()[i] as char);
            }
        } else if c as u8 == VariablePlaceholder::Integer as u8 {
            // Integer variable
            if encoded_var_index < encoded_vars.len() {
                decode_integer_var_into(encoded_vars[encoded_var_index], temp_buffer);
                message_buffer.push_str(temp_buffer);
                temp_buffer.clear(); // Reuse for next variable
                encoded_var_index += 1;
            }
        } else if c as u8 == VariablePlaceholder::Float as u8 {
            // Float variable
            if encoded_var_index < encoded_vars.len() {
                decode_float_var_into(encoded_vars[encoded_var_index], temp_buffer);
                message_buffer.push_str(temp_buffer);
                temp_buffer.clear(); // Reuse for next variable
                encoded_var_index += 1;
            }
        } else if c as u8 == VariablePlaceholder::Dictionary as u8 {
            // Dictionary variable
            if dictionary_var_index < dictionary_vars.len() {
                message_buffer.push_str(&dictionary_vars[dictionary_var_index]);
                dictionary_var_index += 1;
            }
        } else {
            // Regular character
            message_buffer.push(c);
        }

        i += 1;
    }
}

/// Decode an integer variable into a provided buffer
fn decode_integer_var_into<T: EncodedVariable>(encoded_var: T, buffer: &mut String) {
    if std::mem::size_of::<T>() == 8 {
        use std::fmt::Write;
        write!(buffer, "{}", T::as_u64(encoded_var.to_bits()) as i64).unwrap();
    } else {
        use std::fmt::Write;
        write!(buffer, "{}", T::as_u32(encoded_var.to_bits()) as i32).unwrap();
    }
}

/// Decode a float variable into a provided buffer
fn decode_float_var_into<T: EncodedVariable>(encoded_var: T, buffer: &mut String) {
    let mut is_negative = false;
    // Initialize digits based on the type
    let mut digits: T::DigitsType;
    if std::mem::size_of::<T::DigitsType>() == 8 {
        digits = T::from_u64(0);
    } else {
        digits = T::from_u32(0);
    }
    let mut num_digits: u8 = 0;
    let mut decimal_point_pos: u8 = 0;

    decode_float_properties(
        encoded_var,
        &mut is_negative,
        &mut digits,
        &mut num_digits,
        &mut decimal_point_pos,
    );

    // Build the number string in the buffer
    if is_negative {
        buffer.push('-');
    }

    // Convert digits to string representation
    let digits_value = if std::mem::size_of::<T::DigitsType>() == 8 {
        T::as_u64(digits)
    } else {
        T::as_u32(digits) as u64
    };

    // Create the digit string with proper formatting
    let mut digits_str = digits_value.to_string();

    // Ensure we have enough leading zeros if needed
    while digits_str.len() < num_digits as usize {
        digits_str.insert(0, '0');
    }

    // Insert decimal point at the correct position
    let decimal_pos = digits_str.len() - decimal_point_pos as usize;
    if decimal_pos < digits_str.len() && decimal_point_pos > 0 {
        digits_str.insert(decimal_pos, '.');
    }

    buffer.push_str(&digits_str);
}

/// Thread-local decoding context for better performance in single-threaded scenarios
thread_local! {
    static THREAD_LOCAL_DECODE_CONTEXT: std::cell::RefCell<DecodingContext> =
        std::cell::RefCell::new(DecodingContext::new(2048, 128));
}

/// High-performance decode function using thread-local context
pub fn decode_message_fast<T: EncodedVariable>(
    logtype: &str,
    encoded_vars: &[T],
    dictionary_vars: &[String],
) -> String {
    THREAD_LOCAL_DECODE_CONTEXT.with(|context| {
        let mut ctx = context.borrow_mut();
        ctx.decode_message(logtype, encoded_vars, dictionary_vars)
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use crate::extraction::clp::core::{EightByteEncodedVariable, decode_message, encode_message};

    use super::*;
    #[test]
    fn test_decoding_context() {
        let original = "User ID=123 logged in from 192.168.1.1 with balance 45.67";
        let (logtype, encoded_vars, dictionary_vars) =
            encode_message::<EightByteEncodedVariable>(original);

        let mut context = DecodingContext::new(1024, 64);
        let decoded = context.decode_message::<EightByteEncodedVariable>(
            &logtype,
            &encoded_vars,
            &dictionary_vars,
        );

        assert_eq!(decoded, original);
        assert_eq!(context.stats().total_processed, 1);
    }

    #[test]
    fn test_context_reuse() {
        let mut context = DecodingContext::new(1024, 64);

        // Process multiple messages to test reuse
        for i in 0..100 {
            let logtype = "Message \x11: \x12";
            let encoded_vars = vec![i as i64];
            let dictionary_vars = vec![format!("test{}", i)];

            let _decoded = context.decode_message::<EightByteEncodedVariable>(
                logtype,
                &encoded_vars,
                &dictionary_vars,
            );
        }

        let stats = context.stats();
        assert_eq!(stats.total_processed, 100);
        println!("Final stats after 100 decodes: {:?}", stats);
    }

    #[test]
    fn benchmark_comparison() {
        use std::time::Instant;

        let logtype = "User ID=\x11 logged in from \x12 with balance \x13";
        let encoded_vars = vec![123i64, 456i64, 789i64];
        let dictionary_vars = vec!["192.168.1.1".to_string()];

        // Benchmark with context reuse
        let mut context = DecodingContext::new(1024, 64);
        let start = Instant::now();
        for _ in 0..10000 {
            let _decoded = context.decode_message::<EightByteEncodedVariable>(
                logtype,
                &encoded_vars,
                &dictionary_vars,
            );
        }
        let context_time = start.elapsed();

        // Benchmark without context (original method)
        let start = Instant::now();
        for _ in 0..10000 {
            let _decoded = decode_message::<EightByteEncodedVariable>(
                logtype,
                &encoded_vars,
                &dictionary_vars,
            );
        }
        let no_context_time = start.elapsed();

        println!("With context: {:?}", context_time);
        println!("Without context: {:?}", no_context_time);
        println!(
            "Speedup: {:.2}x",
            no_context_time.as_nanos() as f64 / context_time.as_nanos() as f64
        );
    }

    #[test]
    fn test_decode_message_into_matches_decode_message() {
        let messages = vec![
            "Simple message with no variables",
            "Error: code=404 at 10.0.1.15",
            "Rate: 3.14 req/sec from host abc123",
            "",
            "Unicode: hello world test",
        ];

        for msg in &messages {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(msg);

            // Decode with core::decode_message
            let decoded_core = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );

            // Decode with decode_message_into
            let mut message_buffer = String::new();
            let mut temp_buffer = String::new();
            decode_message_into::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
                &mut message_buffer,
                &mut temp_buffer,
            );

            assert_eq!(
                decoded_core, message_buffer,
                "Mismatch for message: {:?}",
                msg
            );
        }
    }
}
