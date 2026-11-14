use std::mem;

/// Variable placeholder types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VariablePlaceholder {
    Integer = 0x11,
    Dictionary = 0x12,
    Float = 0x13,
    Escape = b'\\',
}

/// Constants for bit masks
pub const FOUR_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK: u32 = (1 << 25) - 1;
pub const EIGHT_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK: u64 = (1 << 54) - 1;

/// Type aliases to match C++ implementation
pub type EightByteEncodedVariable = i64;
pub type FourByteEncodedVariable = i32;

/// A generic type for encoded variables - can be i32 or i64
pub trait EncodedVariable: Copy + Sized + 'static {
    type DigitsType: Copy + Sized + 'static;
    const DIGITS_BIT_MASK: Self::DigitsType;
    const MAX_REPRESENTABLE_DIGITS: usize;

    fn from_bits(bits: Self::DigitsType) -> Self;
    fn to_bits(self) -> Self::DigitsType;
    fn as_u64(digits: Self::DigitsType) -> u64;
    fn as_u32(digits: Self::DigitsType) -> u32;
    fn from_u64(val: u64) -> Self::DigitsType;
    fn from_u32(val: u32) -> Self::DigitsType;
}

impl EncodedVariable for FourByteEncodedVariable {
    type DigitsType = u32;
    const DIGITS_BIT_MASK: u32 = FOUR_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK;
    const MAX_REPRESENTABLE_DIGITS: usize = 8;

    fn from_bits(bits: u32) -> Self {
        unsafe { mem::transmute(bits) }
    }

    fn to_bits(self) -> u32 {
        unsafe { mem::transmute(self) }
    }

    fn as_u64(digits: Self::DigitsType) -> u64 {
        digits as u64
    }

    fn as_u32(digits: Self::DigitsType) -> u32 {
        digits
    }

    fn from_u64(val: u64) -> Self::DigitsType {
        val as u32
    }

    fn from_u32(val: u32) -> Self::DigitsType {
        val
    }
}

impl EncodedVariable for EightByteEncodedVariable {
    type DigitsType = u64;
    const DIGITS_BIT_MASK: u64 = EIGHT_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK;
    const MAX_REPRESENTABLE_DIGITS: usize = 16;

    fn from_bits(bits: u64) -> Self {
        unsafe { mem::transmute(bits) }
    }

    fn to_bits(self) -> u64 {
        unsafe { mem::transmute(self) }
    }

    fn as_u64(digits: Self::DigitsType) -> u64 {
        digits
    }

    fn as_u32(digits: Self::DigitsType) -> u32 {
        digits as u32
    }

    fn from_u64(val: u64) -> Self::DigitsType {
        val
    }

    fn from_u32(val: u32) -> Self::DigitsType {
        val as u64
    }
}

/// Function to check if a character is a delimiter
fn is_delim(c: char) -> bool {
    // Everything except "+-.0-9A-Z\_a-z" is a delimiter
    !(c == '+'
        || c == '-'
        || c == '.'
        || c.is_ascii_digit()
        || c.is_ascii_uppercase()
        || c == '_'
        || c.is_ascii_lowercase())
}

/// Function to check if a character is a variable placeholder
fn is_variable_placeholder(c: u8) -> bool {
    c == VariablePlaceholder::Integer as u8
        || c == VariablePlaceholder::Dictionary as u8
        || c == VariablePlaceholder::Float as u8
}

/// Function to check if a string could be a multi-digit hex value
fn could_be_multi_digit_hex_value(s: &str) -> bool {
    if s.len() < 2 {
        return false;
    }

    s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Function to get the bounds of the next variable in a string
pub fn get_bounds_of_next_var(
    msg: &str,
    mut begin_pos: usize,
    mut end_pos: usize,
) -> Option<(usize, usize)> {
    // Ensure we start from a valid character boundary
    if end_pos > msg.len() {
        return None;
    }

    // If end_pos is not on a character boundary, find the next one
    while end_pos < msg.len() && !msg.is_char_boundary(end_pos) {
        end_pos += 1;
    }

    loop {
        begin_pos = end_pos;

        // Find next non-delimiter using char_indices for proper UTF-8 handling
        let mut found_non_delim = false;
        for (offset, c) in msg[begin_pos..].char_indices() {
            if !is_delim(c) {
                begin_pos = begin_pos + offset;
                found_non_delim = true;
                break;
            }
        }

        if !found_non_delim {
            // Reached end of string (only delimiters remaining)
            return None;
        }

        let mut contains_decimal_digit = false;
        let mut contains_alphabet = false;

        // Find next delimiter - start from begin_pos
        end_pos = begin_pos;
        for (offset, c) in msg[begin_pos..].char_indices() {
            if is_delim(c) {
                let is_microsecond_suffix = matches!(c, 'µ' | 'μ')
                    && msg[begin_pos..begin_pos + offset].parse::<f64>().is_ok()
                    && msg[begin_pos + offset + c.len_utf8()..]
                        .strip_prefix('s')
                        .is_some_and(|tail| tail.chars().next().is_none_or(is_delim));
                if !is_microsecond_suffix {
                    end_pos = begin_pos + offset;
                    break;
                }
            }
            if c.is_ascii_digit() {
                contains_decimal_digit = true;
            } else if c.is_ascii_alphabetic() {
                contains_alphabet = true;
            }
            // Update end_pos to include this character
            end_pos = begin_pos + offset + c.len_utf8();
        }

        let variable = &msg[begin_pos..end_pos];

        // Check if preceded by '=' character
        let preceded_by_equals = if begin_pos > 0 {
            // Get the character just before begin_pos
            msg[..begin_pos].chars().last() == Some('=')
        } else {
            false
        };

        // Treat token as variable if:
        // - it contains a decimal digit, or
        // - it's directly preceded by '=' and contains an alphabet char, or
        // - it could be a multi-digit hex value
        if contains_decimal_digit
            || (preceded_by_equals && contains_alphabet)
            || could_be_multi_digit_hex_value(variable)
        {
            break;
        }
    }

    if begin_pos < msg.len() {
        Some((begin_pos, end_pos))
    } else {
        None
    }
}

/// Function to get the bounds of the next variable in a string
// fn get_bounds_of_next_var(msg: &str, mut begin_pos: usize, mut end_pos: usize) -> Option<(usize, usize)> {
//     let msg_chars: Vec<char> = msg.chars().collect();
//     let msg_length = msg_chars.len();

//     if msg_length <= end_pos {
//         return None;
//     }

//     loop {
//         begin_pos = end_pos;

//         // Find next non-delimiter
//         while begin_pos < msg_length && is_delim(msg_chars[begin_pos]) {
//             begin_pos += 1;
//         }

//         if msg_length == begin_pos {
//             // Early exit for performance
//             return None;
//         }

//         let mut contains_decimal_digit = false;
//         let mut contains_alphabet = false;

//         // Find next delimiter
//         end_pos = begin_pos;
//         while end_pos < msg_length && !is_delim(msg_chars[end_pos]) {
//             let c = msg_chars[end_pos];
//             if c.is_ascii_digit() {
//                 contains_decimal_digit = true;
//             } else if c.is_ascii_alphabetic() {
//                 contains_alphabet = true;
//             }
//             end_pos += 1;
//         }

//         let variable = &msg[begin_pos..end_pos];

//         // Treat token as variable if:
//         // - it contains a decimal digit, or
//         // - it's directly preceded by '=' and contains an alphabet char, or
//         // - it could be a multi-digit hex value
//         if contains_decimal_digit ||
//            (begin_pos > 0 && msg_chars[begin_pos - 1] == '=' && contains_alphabet) ||
//            could_be_multi_digit_hex_value(variable) {
//             break;
//         }
//     }

//     if msg_length != begin_pos {
//         Some((begin_pos, end_pos))
//     } else {
//         None
//     }
// }

/// Function to escape and append a constant to the logtype
pub fn escape_and_append_const_to_logtype(constant: &str, logtype: &mut String) {
    append_constant_to_logtype(
        constant,
        |_, _, logtype| {
            logtype.push(VariablePlaceholder::Escape as u8 as char);
        },
        logtype,
    );
}

/// Function to append a constant to the logtype with escaping
pub fn append_constant_to_logtype<F>(constant: &str, mut escape_handler: F, logtype: &mut String)
where
    F: FnMut(&str, usize, &mut String),
{
    let constant_bytes = constant.as_bytes();
    let constant_len = constant_bytes.len();
    let mut begin_pos = 0;

    for i in 0..constant_len {
        let c = constant_bytes[i];
        let is_escape_char = c == VariablePlaceholder::Escape as u8;

        if !is_escape_char && !is_variable_placeholder(c) {
            continue;
        }

        logtype.push_str(&constant[begin_pos..i]);
        begin_pos = i;
        escape_handler(constant, i, logtype);
    }

    logtype.push_str(&constant[begin_pos..constant_len]);
}

/// Function to encode a float string
pub fn encode_float_string<T: EncodedVariable>(s: &str) -> Option<T> {
    if s.is_empty() {
        return None;
    }

    if s.len() > 18 {
        // Anything longer than 18 characters is too big
        return None;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0;
    let value_length = chars.len();

    // Check for negative sign
    let mut is_negative = false;
    if pos < value_length && chars[pos] == '-' {
        is_negative = true;
        pos += 1;
    }

    // Check if there are enough characters for a valid float
    if pos >= value_length {
        return None;
    }

    let mut num_digits = 0;
    let mut decimal_point_pos = None;

    // Initialize digits based on the type
    let mut digits: T::DigitsType;
    if std::mem::size_of::<T::DigitsType>() == 8 {
        digits = T::from_u64(0);
    } else {
        digits = T::from_u32(0);
    }

    while pos < value_length {
        let c = chars[pos];
        if c.is_ascii_digit() {
            let digit_val = (c as u8 - b'0') as u32;

            // Handle the digit based on the specific DigitsType
            if std::mem::size_of::<T::DigitsType>() == 8 {
                // For u64
                let current = T::as_u64(digits);
                digits = T::from_u64(current.checked_mul(10)?.checked_add(digit_val as u64)?);
            } else {
                // For u32
                let current = T::as_u32(digits);
                digits = T::from_u32(current.checked_mul(10)?.checked_add(digit_val)?);
            }

            num_digits += 1;
        } else if decimal_point_pos.is_none() && c == '.' {
            decimal_point_pos = Some(value_length - 1 - pos);
        } else {
            // Invalid character
            return None;
        }
        pos += 1;
    }

    // Validate decimal point position
    if decimal_point_pos.is_none() || decimal_point_pos.unwrap() == 0 || num_digits == 0 {
        return None;
    }

    let decimal_point_pos = decimal_point_pos.unwrap();

    // Check if digits are within representable range
    if std::mem::size_of::<T::DigitsType>() == 8 {
        if T::as_u64(digits) > T::as_u64(T::DIGITS_BIT_MASK) {
            return None;
        }
    } else {
        if T::as_u32(digits) > T::as_u32(T::DIGITS_BIT_MASK) {
            return None;
        }
    }

    encode_float_properties::<T>(is_negative, digits, num_digits, decimal_point_pos)
}

/// Function to encode float properties
fn encode_float_properties<T: EncodedVariable>(
    is_negative: bool,
    digits: T::DigitsType,
    num_digits: usize,
    decimal_point_pos: usize,
) -> Option<T> {
    if !(1..=T::MAX_REPRESENTABLE_DIGITS).contains(&num_digits)
        || !(1..=num_digits).contains(&decimal_point_pos)
        || T::as_u64(digits) > T::as_u64(T::DIGITS_BIT_MASK)
    {
        return None;
    }
    if std::mem::size_of::<T>() == 8 {
        // 64-bit encoding
        let mut encoded_float: u64 = 0;

        if is_negative {
            encoded_float = 1;
        }

        encoded_float <<= 55; // 1 unused + 54 for digits

        // Convert digits to u64
        let digits_u64 = T::as_u64(digits);
        encoded_float |= digits_u64 & EIGHT_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK;

        encoded_float <<= 4;
        encoded_float |= ((num_digits - 1) & 0x0F) as u64;
        encoded_float <<= 4;
        encoded_float |= ((decimal_point_pos - 1) & 0x0F) as u64;

        // Convert to T
        Some(T::from_bits(T::from_u64(encoded_float)))
    } else {
        // 32-bit encoding
        let mut encoded_float: u32 = 0;

        if is_negative {
            encoded_float = 1;
        }

        encoded_float <<= 25; // 25 for digits

        // Convert digits to u32
        let digits_u32 = T::as_u32(digits);
        encoded_float |= digits_u32 & FOUR_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK;

        encoded_float <<= 3;
        encoded_float |= ((num_digits - 1) & 0x07) as u32;
        encoded_float <<= 3;
        encoded_float |= ((decimal_point_pos - 1) & 0x07) as u32;

        // Convert to T
        Some(T::from_bits(T::from_u32(encoded_float)))
    }
}

/// Function to encode an integer string
pub fn encode_integer_string<T: EncodedVariable>(s: &str) -> Option<T> {
    if s.is_empty() {
        return None;
    }

    let chars: Vec<char> = s.chars().collect();

    // Ensure start of value is an integer with no zero-padding or positive sign
    if chars[0] == '-' {
        // Ensure first character after sign is a non-zero integer
        if chars.len() < 2 || !chars[1].is_ascii_digit() || chars[1] == '0' {
            return None;
        }
    } else {
        // Ensure first character is a digit
        if !chars[0].is_ascii_digit() {
            return None;
        }

        // Ensure value is not zero-padded
        if chars.len() > 1 && chars[0] == '0' {
            return None;
        }
    }

    let mut start_pos = 0;
    if chars[0] == '-' {
        start_pos = 1;
    };

    for (_i, &c) in chars.iter().enumerate().skip(start_pos) {
        if !c.is_ascii_digit() {
            return None; // Reject if ANY non-digit character is found
        }
    }

    // Convert string to integer
    match s.parse::<i64>() {
        Ok(val) if std::mem::size_of::<T>() == 8 => Some(T::from_bits(T::from_u64(val as u64))),
        Ok(val)
            if std::mem::size_of::<T>() == 4
                && val >= i32::MIN as i64
                && val <= i32::MAX as i64 =>
        {
            Some(T::from_bits(T::from_u32(val as i32 as u32)))
        }
        _ => None,
    }
}

/// Main function to encode a message
///
/// This function takes a log message and processes it to extract variable parts:
/// - Integer variables (whole numbers)
/// - Float variables (numbers with decimal points)
/// - Dictionary variables (other variable types)
///
/// It returns:
/// - logtype: A string template with variable placeholders
/// - encoded_vars: Vector of encoded numeric variables (integers and floats)
/// - dictionary_vars: Vector of string values for non-numeric variables
pub fn encode_message<T: EncodedVariable>(message: &str) -> (String, Vec<T>, Vec<String>) {
    let mut logtype = String::new();
    let mut encoded_vars = Vec::new();
    let mut dictionary_vars = Vec::new();

    let mut var_begin_pos = 0;
    let mut var_end_pos = 0;

    while let Some((begin_pos, end_pos)) =
        get_bounds_of_next_var(message, var_begin_pos, var_end_pos)
    {
        // Process constant text before this variable
        let constant = &message[var_begin_pos..begin_pos];
        escape_and_append_const_to_logtype(constant, &mut logtype);

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
        escape_and_append_const_to_logtype(constant, &mut logtype);
    }

    (logtype, encoded_vars, dictionary_vars)
}

// /// Main function to encode a message
// ///
// /// This function takes a log message and processes it to extract variable parts:
// /// - Integer variables (whole numbers)
// /// - Float variables (numbers with decimal points)
// /// - Dictionary variables (other variable types)
// ///
// /// It returns:
// /// - logtype: A string template with variable placeholders
// /// - encoded_vars: Vector of encoded numeric variables (integers and floats)
// /// - dictionary_var_bounds: Vector of positions of dictionary variables
// pub fn encode_message<T: EncodedVariable>(message: &str) -> (String, Vec<T>, Vec<i32>) {
//     let mut logtype = String::new();
//     let mut encoded_vars = Vec::new();
//     let mut dictionary_var_bounds = Vec::new();

//     let mut var_begin_pos = 0;
//     let mut var_end_pos = 0;

//     while let Some((begin_pos, end_pos)) = get_bounds_of_next_var(message, var_begin_pos, var_end_pos) {
//         // Process constant text before this variable
//         let constant = &message[var_begin_pos..begin_pos];
//         escape_and_append_const_to_logtype(constant, &mut logtype);

//         // Update positions
//         var_begin_pos = begin_pos;
//         var_end_pos = end_pos;

//         // Process the variable
//         let var_string = &message[var_begin_pos..var_end_pos];

//         if let Some(encoded_var) = encode_float_string::<T>(var_string) {
//             // Float variable
//             logtype.push(VariablePlaceholder::Float as u8 as char);
//             encoded_vars.push(encoded_var);
//         } else if let Some(encoded_var) = encode_integer_string::<T>(var_string) {
//             // Integer variable
//             logtype.push(VariablePlaceholder::Integer as u8 as char);
//             encoded_vars.push(encoded_var);
//         } else {
//             // Dictionary variable
//             logtype.push(VariablePlaceholder::Dictionary as u8 as char);
//             dictionary_var_bounds.push(var_begin_pos as i32);
//             dictionary_var_bounds.push(var_end_pos as i32);
//         }

//         // Move to position after this variable
//         var_begin_pos = var_end_pos;
//     }

//     // Process any remaining constant text
//     if var_begin_pos < message.len() {
//         let constant = &message[var_begin_pos..];
//         escape_and_append_const_to_logtype(constant, &mut logtype);
//     }

//     (logtype, encoded_vars, dictionary_var_bounds)
// }

/// Function to decode a float variable
fn decode_float_var<T: EncodedVariable>(encoded_var: T) -> String {
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

    // Extract digits as a string
    let mut digits_str = if std::mem::size_of::<T::DigitsType>() == 8 {
        T::as_u64(digits).to_string()
    } else {
        T::as_u32(digits).to_string()
    };

    // Ensure we have enough leading zeros if needed
    while digits_str.len() < num_digits as usize {
        digits_str.insert(0, '0');
    }

    // Insert decimal point
    let decimal_pos = digits_str.len() - decimal_point_pos as usize;
    if decimal_pos < digits_str.len() {
        digits_str.insert(decimal_pos, '.');
    }

    // Add negative sign if needed
    if is_negative {
        digits_str.insert(0, '-');
    }

    digits_str
}

/// Function to decode float properties
pub fn decode_float_properties<T: EncodedVariable>(
    encoded_var: T,
    is_negative: &mut bool,
    digits: &mut T::DigitsType,
    num_digits: &mut u8,
    decimal_point_pos: &mut u8,
) {
    if std::mem::size_of::<T>() == 8 {
        // 64-bit decoding
        let encoded_float = T::as_u64(encoded_var.to_bits());

        // Extract according to the format described in encode_float_properties
        *decimal_point_pos = ((encoded_float & 0x0F) + 1) as u8;
        let shifted = encoded_float >> 4;
        *num_digits = ((shifted & 0x0F) + 1) as u8;
        let shifted = shifted >> 4;
        *digits = T::from_u64(shifted & EIGHT_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK);
        let shifted = shifted >> 55;
        *is_negative = shifted > 0;
    } else {
        // 32-bit decoding
        let encoded_float = T::as_u32(encoded_var.to_bits());

        // Extract according to the format described in encode_float_properties
        *decimal_point_pos = ((encoded_float & 0x07) + 1) as u8;
        let shifted = encoded_float >> 3;
        *num_digits = ((shifted & 0x07) + 1) as u8;
        let shifted = shifted >> 3;
        *digits = T::from_u32(shifted & FOUR_BYTE_ENCODED_FLOAT_DIGITS_BIT_MASK);
        let shifted = shifted >> 25;
        *is_negative = shifted > 0;
    }
}

/// Function to decode an integer variable
fn decode_integer_var<T: EncodedVariable>(encoded_var: T) -> String {
    if std::mem::size_of::<T>() == 8 {
        (T::as_u64(encoded_var.to_bits()) as i64).to_string()
    } else {
        (T::as_u32(encoded_var.to_bits()) as i32).to_string()
    }
}

/// Function to decode a message from its encoded components
pub fn decode_message<T: EncodedVariable>(
    logtype: &str,
    encoded_vars: &[T],
    dictionary_vars: &[String],
) -> String {
    // Pre-allocate with a reasonable capacity to reduce reallocations
    let mut message = String::with_capacity(logtype.len());
    let mut encoded_var_index = 0;
    let mut dictionary_var_index = 0;

    // Use a char iterator to correctly handle multi-byte UTF-8 characters
    let mut chars = logtype.chars();
    while let Some(c) = chars.next() {
        if c == VariablePlaceholder::Escape as u8 as char {
            // This is an escape character, so the next character should be taken literally.
            // The next char from the iterator is the one to append.
            if let Some(escaped_char) = chars.next() {
                message.push(escaped_char);
            }
        } else if c == VariablePlaceholder::Integer as u8 as char {
            // Integer variable
            if encoded_var_index < encoded_vars.len() {
                message.push_str(&decode_integer_var(encoded_vars[encoded_var_index]));
                encoded_var_index += 1;
            }
        } else if c == VariablePlaceholder::Float as u8 as char {
            // Float variable
            if encoded_var_index < encoded_vars.len() {
                message.push_str(&decode_float_var(encoded_vars[encoded_var_index]));
                encoded_var_index += 1;
            }
        } else if c == VariablePlaceholder::Dictionary as u8 as char {
            // Dictionary variable
            if dictionary_var_index < dictionary_vars.len() {
                message.push_str(&dictionary_vars[dictionary_var_index]);
                dictionary_var_index += 1;
            }
        } else {
            // Regular character
            message.push(c);
        }
    }

    message
}
// pub fn decode_message<T: EncodedVariable>(
//     logtype: &str,
//     encoded_vars: &[T],
//     dictionary_vars: &[String]
// ) -> String {
//     let mut message = String::new();
//     let mut encoded_var_index = 0;
//     let mut dictionary_var_index = 0;

//     // Process each character in the logtype
//     let mut i = 0;
//     while i < logtype.len() {
//         let c = logtype.as_bytes()[i] as char;

//         if c as u8 == VariablePlaceholder::Escape as u8 {
//             // This is an escape character, so the next character should be taken literally
//             i += 1;
//             if i < logtype.len() {
//                 message.push(logtype.as_bytes()[i] as char);
//             }
//         } else if c as u8 == VariablePlaceholder::Integer as u8 {
//             // Integer variable
//             if encoded_var_index < encoded_vars.len() {
//                 message.push_str(&decode_integer_var(encoded_vars[encoded_var_index]));
//                 encoded_var_index += 1;
//             }
//         } else if c as u8 == VariablePlaceholder::Float as u8 {
//             // Float variable
//             if encoded_var_index < encoded_vars.len() {
//                 message.push_str(&decode_float_var(encoded_vars[encoded_var_index]));
//                 encoded_var_index += 1;
//             }
//         } else if c as u8 == VariablePlaceholder::Dictionary as u8 {
//             // Dictionary variable
//             if dictionary_var_index < dictionary_vars.len() {
//                 message.push_str(&dictionary_vars[dictionary_var_index]);
//                 dictionary_var_index += 1;
//             }
//         } else {
//             // Regular character
//             message.push(c);
//         }

//         i += 1;
//     }

//     message
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_encode_and_decode() {
        let log_message0 = "INFO: rate(1min) = 10.9±36.2/sec; rate(5min) = 83.0±67.6/sec; rate(15min) = 117.2±50.4/sec; rate(total) = 8.0±27.7/sec; N = 33";
        let log_message1 = "I, [1992-06-13T22:54:52.068683 #4]  INFO -- : [Inventory Sync Worker] Batch started for widget orders, group: demo, request_id: 6499a8a5-baa8-92d8-cfb7-ab2adbc16e24, optional_code: , optional_tag: ";
        let log_message2 = "I, [1949-06-07T15:54:47.495049 #4]  INFO -- : Performing Jobs::Inventory::OrderSyncWorker 55ce669b226514c88c539ad4 with arguments [\"---\\n- !ruby/object 'Jobs::Inventory::SyncTask'\\n- :record\\n- - - 3c94bc42-e84c-e975-ae07-9cf9238f6264\\n- {}\\n\"] at 1982-08-28 20:15:21 +1030";
        let log_message3 = "I, [1988-10-28T13:27:51.943108 #4]  INFO -- : Performing OrderSyncWorker 87d914df85f9133c8bee9612 with arguments [{\"payload\"=>\"eJyN1VuvqkYYgOH/wm2XK8wgoN4hICqiyEGRptnhMALK+Sw767/Xld02TVMy\\n3M43GR6+m/cnkZU+Kn9EPrEiTpogajOdU9SDOCNJlvggOqdEYdZU6D1GvZPk\\nMZpVdVai9yzMqvp9HKUtSt9Hr8+/Lnx6WfIeo+/jilj9/pOIapT8+sR1J0ii\\nMSMBSf595X36y1CiCpUt8t+DonHSOqpfxAp+EHWUoKp+P/2+CUkKzEh2Ri4M\\nAFfUfAXITwCpOc2wi+VvJL2ivt8t0R2VKPW+1dm2OyjIsYtsKdjC8XEEkqH6\\nF7lygaBDnX5Za+Lr4/+N4F/Gf37zR5P7Tv0fJoVjAgyTgUGUsmul2vStLM/P\\nNFiqT8DO62pq70ufXN6tUSactso5zggxxuFRXaCneHoDF5tljQ5uMhweHqhZ\\nquelo7xTmFEjNXmVNI5JYZgbTs+qhbRzhr2rHH2FP7ea4foqm/Ny3dWdXPWj\\nzPm0VTI44xxjvLh7o06tLOa0oG7sNXg2J7hOQym+2XF5O1ptNGqkJ6+SxTFp\\nDLOyinbgBvrhMHDJMXRq2qpUthmSy5s312iK2o8ymWmrXOCMDMbomM9KlQbm\\n0OZAuLbJ9va0aMt2FOAXhcQJV8cfNbKTV7nEMVkMMwzRgrxv6ro9yV3chDu9\\nR3F/uPaWwvWixml8PspcTFslIHHIBQa51RDHQaanrIIPbyplqcYukJ3o6t0D\\nGLtkSKqjyOXkXQKAcy4xTpvdW95lowcm7Tc7O49zxXx1r3a4bxaD+TibqjTm\\nBBPDA3DlgbjyCEfqcTfn0lqPi0SuszNbaQMpF6kr9M+uvkZLYRQ5vTwAlx6I\\nS89BsG87QQcvg+WzTFdZMo+LWANUDmrOuN/tqht1TkwPwLUH4tqzV51mLaYy\\n+9qGpRpQ84DpL2alnqzCZNVtKLb6KHJ6ewAuPhAXn/M+Ph1j6hTSUaMfdie+\\nfiLd3Bpd0UNvk1/aZhh1TowPwNUH4uqzvbRkUATB1YjjwbyZ8uOVSPskv7oW\\nxfQ7+bg4jyKn1wfg8gOx+QHiwXAuqijxseRtTus9S0WW4ncZ2tIN0CJwGHVO\\nzA/A9Qfi+kPrRnujwdoUAw8lD1HbcqeKN18xpXe9JKdOch1FTu8PwAUI4gIU\\n8UnyHJ5u8BR9dz1o/vBy2fQuPJU29I3yytMV8fXH15/uZ6aR\\n\", \"success\"=>4, \"error\"=>nil, \"optional_result\"=>nil, \"optional_note\"=>nil, \"enabled\"=>false, \"archived\"=>true, \"record_key\"=>\"widget\", \"optional_code\"=>nil, \"optional_tag\"=>nil, \"ready\"=>true, \"quantity\"=>3, \"created_at\"=>\"1982-08-28 20:15:21 +1015\", \"mode\"=>\"false\", \"active\"=>false, \"metadata\"=>{\"category\"=>\"inventory\", \"job_key\"=>\"9e53a73e-c514-df38-6707-95d616b02d45\", \"source_key\"=>\"ITEM7654321\", \"item_code\"=>\"WIDGET\", \"optional_batch\"=>nil, \"mode\"=>false, \"event_key\"=>\"InventoryUpdated\", \"verified\"=>true, \"labels\"=>[\"queued\", \"ready\", \"stored\"], \"group_key\"=>\"EXAMPLE\", \"available\"=>false, \"updated_at\"=>\"1897-09-28T22:44:18.821+05:30\", \"active\"=>false, \"active_ab\"=>[], \"options\"=>{\"queue\"=>\"inventory-high-processor\", \"queue_item\"=>nil, \"optional_value\"=>nil, \"optional_limit\"=>nil, \"optional_range\"=>{}}, \"extras\"=>nil, \"optional_status\"=>false, \"visible\"=>nil, \"optional_phase\"=>nil, \"optional_owner\"=>nil, \"optional_zone\"=>\"synthetic\", \"environment\"=>\"sandbox\", \"worker\"=>\"a1b2c3d4e5\", \"processor\"=>true, \"accepted\"=>4, \"priority\"=>5, \"retries\"=>true}, \"environment\"=>\"sandbox\", \"updated_at\"=>\"1897-09-28T22:44:18.821+05:30\", \"complete\"=>\"1982-08-28 20:15:21 +1015\", \"worker_av\"=>Tue, 05 Jun 1928 10:20:45.951827954 IST +05:45}, \"9e53a73e-c514-df38-6707-95d616b02d45\"] at 1982-08-28 20:15:21 +1030";
        let log_message4 = "I, [1947-05-18T18:26:25.924657 #4]  INFO -- : Inventory Batch {:updated_at_aw=>[], :received_at=>[{\"category_item\"=>\"InventoryUpdated\", \"item_az\"=>{\"payload\"=>\"eJyN1VuvqkYYgOH/wm2XK8wgoN4hICqiyEGRptnhMALK+Sw767/Xld02TVMy\\n3M43GR6+m/cnkZU+Kn9EPrEiTpogajOdU9SDOCNJlvggOqdEYdZU6D1GvZPk\\nMZpVdVai9yzMqvp9HKUtSt9Hr8+/Lnx6WfIeo+/jilj9/pOIapT8+sR1J0ii\\nMSMBSf595X36y1CiCpUt8t+DonHSOqpfxAp+EHWUoKp+P/2+CUkKzEh2Ri4M\\nAFfUfAXITwCpOc2wi+VvJL2ivt8t0R2VKPW+1dm2OyjIsYtsKdjC8XEEkqH6\\nF7lygaBDnX5Za+Lr4/+N4F/Gf37zR5P7Tv0fJoVjAgyTgUGUsmul2vStLM/P\\nNFiqT8DO83pq70ufXN6tUSactso5zggxxuFRXaCneHoDF5tljQ5uMhweHqhZ\\nquelo7xTmFEjNXmVNI5JYZgbTs+qhbRzhr2rHH2FP7ea4foqm/Ny3dWdXPWj\\nzPm0VTI44xxjvLh7o06tLOa0oG7sNXg2J7hOQym+2XF5O1ptNGqkJ6+SxTFp\\nDLOyinbgBvrhMHDJMXRq2qpUthmSy5s312iK2o8ymWmrXOCMDMbomM9KlQbm\\n0OZAuLbJ9va0aMt2FOAXhcQJV8cfNbKTV7nEMVkMMwzRgrxv6ro9yV3chDu9\\nR3F/uPaWwvWixml8PspcTFslIHHIBQa51RDHQaanrIIPbyplqcYukJ3o6t0D\\nGLtkSKqjyOXkXQKAcy4xTpvdW95lowcm7Tc7O49zxXx1r3a4bxaD+TibqjTm\\nBBPDA3DlgbjyCEfqcTfn0lqPi0SuszNbaQMpF6kr9M+uvkZLYRQ5vTwAlx6I\\nS89BsG87QQcvg+WzTFdZMo+LWANUDmrOuN/tqht1TkwPwLUH4tqzV51mLaYy\\n+9qGpRpQ84DpL2alnqzCZNVtKLb6KHJ6ewAuPhAXn/M+Ph1j6hTSUaMfdie+\\nfiLd3Bpd0UNvk1/aZhh1TowPwNUH4uqzvbRkUATB1YjjwbyZ8uOVSPskv7oW\\nxfQ7+bg4jyKn1wfg8gOx+QHiwXAuqijxseRtTus9S0WW4ncZ2tIN0CJwGHVO\\nzA/A9Qfi+kPrRnujwdoUAw8lD1HbcqeKN18xpXe9JKdOch1FTu8PwAUI4gIU\\n8UnyHJ5u8BR9dz1o/vBy2fQuPJU29I3yytMV8fXH15/uZ6aR\\n\", \"success\"=>4, \"error\"=>nil, \"optional_result\"=>nil, \"optional_note\"=>nil, \"enabled\"=>false, \"archived\"=>true, \"record_key\"=>\"widget\", \"optional_code\"=>nil, \"optional_tag\"=>nil, \"ready\"=>true, \"quantity\"=>3, \"created_at\"=>\"1982-08-28 20:15:21 +1015\", \"mode\"=>\"false\", \"active\"=>false, \"metadata\"=>{\"category\"=>\"inventory\", \"job_key\"=>\"9e53a73e-c514-df38-6707-95d616b02d45\", \"source_key\"=>\"ITEM7654321\", \"item_code\"=>\"WIDGET\", \"optional_batch\"=>nil, \"mode\"=>false, \"event_key\"=>\"InventoryUpdated\", \"verified\"=>true, \"labels\"=>[\"queued\", \"ready\", \"stored\"], \"group_key\"=>\"EXAMPLE\", \"available\"=>false, \"updated_at\"=>\"1897-09-28T22:44:18.821+05:30\", \"active\"=>false, \"active_ab\"=>[], \"options\"=>{\"queue\"=>\"inventory-high-processor\", \"queue_item\"=>nil, \"optional_value\"=>nil, \"optional_limit\"=>nil, \"optional_range\"=>{}}, \"extras\"=>nil, \"optional_status\"=>false, \"visible\"=>nil, \"optional_phase\"=>nil, \"optional_owner\"=>nil, \"optional_zone\"=>\"synthetic\", \"environment\"=>\"sandbox\", \"worker\"=>\"a1b2c3d4e5\", \"processor\"=>true, \"accepted\"=>4, \"priority\"=>5, \"retries\"=>true}, \"environment\"=>\"sandbox\", \"updated_at\"=>\"1897-09-28T22:44:18.821+05:30\", \"complete\"=>\"1982-08-28 20:15:21 +1015\"}}], :item_ba=>[], :item_bb2=>[], :item_bc=>[], :item_bd=>[], :item_be=>[], :item_bf=>[], :item_bg=>[], :item_bh=>[], :item_bi=>[], :item_bj=>[], :item_bk=>[], :item_bl=>[], :item_bm=>[], :item_bn=>[]}";
        let log_message5 = "[Sat Jun 07 20:40:43 2025] [nulla:notice] [pid 3701:tid 471] [client 105.207.158.142:26616] Try to hack the TCP alarm, maybe it will generate the primary interface!";
        let log_message6 = "15.154.252.44 - - [07/Jun/2025:20:41:10 +0530] \"POST /communities/syndicate HTTP/2.0\" 403 1036";
        let log_message7 = "32.119.198.236 - romaguera7737 [07/Jun/2025:20:42:37 +0530] \"POST /monetize/harness/deliver/partnerships HTTP/2.0\" 204 38852 \"https://www.centralarchitectures.org/efficient\" \"Mozilla/5.0 (Windows NT 6.2) AppleWebKit/5332 (KHTML, like Gecko) Chrome/40.0.871.0 Mobile Safari/5332\"";
        let log_message8 = "<61>Jun 07 20:43:20 schneider8168 at[32]: You can't reboot the firewall without compressing the open-source AGP port!";
        let log_message9 = "INFO: Transaction completed successfully ✅. Amount: 150.75";
        let log_message10 = "E, [1883-04-20T10:00:58.424325 #4] ERROR -- : [0e0163f7111b19277e256aca] [a1b2c3d4] Event OrderSyncWorker completed with an inventory update for a sample batch 989.66131212975107 sample_bp [batch:5551b289-fc64-78e2-0886-f52218472a743]";
        let log_message11 = "[<TS>] {var1} (INFO) [InventoryWorker] updated {'view_id':'1f439352-5e26-2e1c-c2d3-c81bdfc08e23','client_group_ids':['f6ca94d8-d965-4cd9-8cd2-9ffea9812196','159e5be8-3f30-8c8e-ad6d-3824a361a1d4','4dcb4057-baba-b9af-b381-516f5b53a6f0','bbc5f2fc-70c4-6eae-5e2a-435ee077173b','8644b3fc-fd61-b409-0978-007ab5caf643'],'attempts':4,'limit':58}";

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message0);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message0); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message1);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message1); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message2);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message2); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message3);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message3); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message4);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message4); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message5);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message5); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message6);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message6); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message7);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message7); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message8);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message8); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message9);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message9); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message10);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message10); // The decoded message should match the original
        }

        {
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(log_message11);
            let decoded_message = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            println!(
                "Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}",
                logtype, encoded_vars, dictionary_vars
            );
            assert_eq!(decoded_message, log_message11); // The decoded message should match the original
        }
    }

    #[test]
    fn test_float_rejects_unrepresentable_digit_counts() {
        for value in [
            "0.0000000000000001",
            ".00000000000000001",
            "-.00000000000000001",
        ] {
            assert!(encode_float_string::<i64>(value).is_none(), "{value}");
            let (template, encoded, dictionary) = encode_message::<i64>(value);
            assert_eq!(dictionary, vec![value]);
            assert_eq!(decode_message(&template, &encoded, &dictionary), value);
        }
        for value in ["0.00000001", ".000000001", "4294967296.0"] {
            assert!(encode_float_string::<i32>(value).is_none(), "{value}");
        }
        for value in [".0000000000000001", "0.000000000000001", "-123.45"] {
            let encoded = encode_float_string::<i64>(value).unwrap();
            assert_eq!(decode_float_var(encoded), value);
        }
        for value in [".00000001", "0.0000001", "-123.45"] {
            assert_eq!(
                decode_float_var(encode_float_string::<i32>(value).unwrap()),
                value
            );
        }
    }

    #[test]
    fn test_signed_integer_roundtrip() {
        for value in [i64::MIN, -1, 0, i64::MAX] {
            let raw = value.to_string();
            let (template, encoded, dictionary) = encode_message::<i64>(&raw);
            assert_eq!(decode_message(&template, &encoded, &dictionary), raw);
        }
        for value in [i32::MIN, -1, 0, i32::MAX] {
            let raw = value.to_string();
            let (template, encoded, dictionary) = encode_message::<i32>(&raw);
            assert_eq!(decode_message(&template, &encoded, &dictionary), raw);
        }
    }
}
