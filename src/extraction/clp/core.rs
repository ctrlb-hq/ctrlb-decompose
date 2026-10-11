
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
        unsafe { u32::cast_signed(bits) }
    }

    fn to_bits(self) -> u32 {
        unsafe { i32::cast_unsigned(self) }
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
        unsafe { u64::cast_signed(bits) }
    }

    fn to_bits(self) -> u64 {
        unsafe { i64::cast_unsigned(self) }
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

/// 256-byte lookup table: true = delimiter, false = non-delimiter.
/// Non-delimiters are: '+', '-', '.', '0'-'9', 'A'-'Z', '_', 'a'-'z'
static DELIM_TABLE: [bool; 256] = {
    let mut table = [true; 256];
    table[b'+' as usize] = false;
    table[b'-' as usize] = false;
    table[b'.' as usize] = false;
    table[b'_' as usize] = false;
    let mut i = b'0';
    while i <= b'9' {
        table[i as usize] = false;
        i += 1;
    }
    i = b'A';
    while i <= b'Z' {
        table[i as usize] = false;
        i += 1;
    }
    i = b'a';
    while i <= b'z' {
        table[i as usize] = false;
        i += 1;
    }
    table
};

/// Function to check if a character is a delimiter (lookup table, branch-free)
#[inline(always)]
fn is_delim(c: char) -> bool {
    let b = c as u32;
    if b < 256 { DELIM_TABLE[b as usize] } else { true }
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
                end_pos = begin_pos + offset;
                break;
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
                digits = T::from_u64(current * 10 + digit_val as u64);
            } else {
                // For u32
                let current = T::as_u32(digits);
                digits = T::from_u32(current * 10 + digit_val);
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

    Some(encode_float_properties::<T>(
        is_negative,
        digits,
        num_digits,
        decimal_point_pos,
    ))
}

/// Function to encode float properties
fn encode_float_properties<T: EncodedVariable>(
    is_negative: bool,
    digits: T::DigitsType,
    num_digits: usize,
    decimal_point_pos: usize,
) -> T {
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
        T::from_bits(T::from_u64(encoded_float))
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
        T::from_bits(T::from_u32(encoded_float))
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

// fn main() {
//     let log_message = r#"[20891d0eb50dc4f567a14301] [6247e714] [New relic Debug] - (job_completed) - {:enqueued_at=>1740957114, :scheduled_at=>nil, :class=>"TrackingCrawlerSqs", :queue=>"data_engine-high-processor", :job_id=>"20891d0eb50dc4f567a14301", :execution_id=>"6247e714", :execution_count=>1, :execution_at=>1740957658, :duration=>14, :status=>"completed", :engine_name=>"container_tracking"}"#;

//     // Test with 32-bit encoding
//     let (logtype, encoded_vars, dictionary_vars) = encode_message::<FourByteEncodedVariable>(log_message);

//     println!("Logtype: {:?}\nEncoded vars: {:?}\nDictionary var bounds: {:?}", logtype, encoded_vars, dictionary_vars);
//     let decoded_message = decode_message::<FourByteEncodedVariable>(&logtype, &encoded_vars, &dictionary_vars);
//     println!("\n\n\n\nDecoded message: {}", decoded_message);
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_encode_and_decode() {
        let log_message0 = "INFO: rate(1min) = 10.9±36.2/sec; rate(5min) = 83.0±67.6/sec; rate(15min) = 117.2±50.4/sec; rate(total) = 8.0±27.7/sec; N = 33";
        let log_message1 = "I, [2025-06-06T17:26:14.759140 #1]  INFO -- : [Live ETL Sync] Message push to sqs queue, model: tracking_trackings, record_id: 79d661d8-773e-4bea-a20a-00ad6ba008e8, request_id: , source: ";
        let log_message2 = "I, [2025-06-06T17:26:14.609043 #1]  INFO -- : Queued Sidekiq::Extensions::DelayedClass 22ca336b993281f55f206fc1 with args [\"---\\n- !ruby/class 'ContainerTracking::EtlFetch::TrackingStats'\\n- :fetch_and_update_data\\n- - - 4e1a78be-a118-4823-9baa-98cbf8098610\\n- {}\\n\"] on 2025-06-06 22:56:14 +0530";
        let log_message3 = "I, [2025-06-06T17:26:14.598333 #1]  INFO -- : Queued TrackingCrawlerSqs 54a681ae52e6800b5fce6389 with args [{\"data\"=>\"eJy9Vm1vmzAQ/ivIn0NlSAgt39yWtWh5E9Bt2jQhB0yCSmxmTKesyn/fkXRp\\nGxLSRd0kPuDz4zs/z/lO94hKRVVVIgeVVRyzskyrPF9GBZUlS6KEKoo66OXK\\neUQJUzTLy/q3EDkcHRB3qF3dkksyuulo4S3xBmR0XR8UCWwHZBSOg4526ZOv\\n3gDMrPaDTGxaOu7r2ESrDooFB6+cybXfof/xzuxedM/tbr3cbkYZT0Vt2XUR\\nYssxLQfjM4wxhCizXwwAPaylCpZqWdTLQFGeUJnUAV+R+gYeHxhXgLkhoauP\\n70I4lYuYqkzwNooLWhTg5s9ptigUyJfF91VRC5BTzjdR2Pa+Pd00Q8N2LPx8\\nXxqriubHcQuR1AAlq/gelg+QMZZHnC7AyiF1z6ZqsbWIJZ2xZ8uqs8PWG51G\\ndga3hYwc4gmfERp9p2u389yHe2+egzG5PonkxvUhft3QgORYR/k1cE/8RMwo\\nfw9+V+NRSLyR62vX7oT44Z3vnpZTIbNZxqOEQX2oSrKW5PZC03CM3lHyDVwb\\neWAShETzvSuiuV8mvhsE6LUYyOpP0Gs1kGWcB2u+NMn47Jj8+4Ujvu99IoPX\\nsgXe6IZMxr7b0ba/TdGUpLyMynlWLMAQUSmzB5q3SHcRml3H6h+VroH7Z9Il\\nWRnPqQTzzh0MHOKeY/X2iXftBfCk/JsTNGutq92Yh/Rp4t67rpp94x3Yrbud\\neaT778P9965x0vN/Q+8AatATescT3MC1FwCZaDBtaEMycEnj8d/al83Hb38+\\n3Dd2E/A3fWN35tmR7EiL2DfS7FNoH+6fKHSgPWzi2y/iv6E9tEvTUjrNaG2q\\n2G9W5c2l830Fg6NQcxhE/wzDislFxulWYHcIvKBMQs0d1i2Y1DwDb+B6PqnH\\nTslSJhmPYXBKYNT9huy0b9DExLrNUkvvTa2+fpFeUJ2m1jmjzEqmNEXfYUCW\\n9GceSabkMsrFbD0mr22g0nQJkcnEA2IxLRRkKkpFxevh+wMZBO6uPZqyVEgW\\nlYzKeL4HVYr8AfxuosVwBBKDO2gq6upWLK5T2diFKZ3DFpzLiqgQUtX0DHxm\\n9I0zfNZFHbNvYgOozEWpnuSC1hHfQ+npGypSn9J6dNaN9QyfsPvsR/SjYtVL\\nbPSEjVJaKrRarX4Dfe/eCA==\\n\", \"success\"=>1, \"error\"=>nil, \"searates_error\"=>nil, \"html_data\"=>nil, \"searates_used\"=>false, \"gocomet_crawler_used\"=>true, \"successfully_parsed_from\"=>\"gocomet_crawler\", \"searates_forced\"=>nil, \"error_type\"=>nil, \"is_compressed\"=>true, \"time_to_crawl\"=>0, \"crawling_start_time\"=>\"2025-06-06 17:26:13 +0000\", \"priority\"=>\"false\", \"paid_api_crawling\"=>false, \"request_data\"=>{\"env\"=>\"production\", \"reference_id\"=>\"7f61ad20-7ef5-4b56-9f9a-af58eae5dbaf\", \"tracking_number\"=>\"MRKU2393873\", \"carrier_code\"=>\"MAEU\", \"login_details\"=>nil, \"priority\"=>false, \"event_type\"=>\"ContainerTrackingUpdate\", \"is_crawled_by_gocomet_crawler\"=>true, \"tracking_types\"=>[\"bk\", \"bl\", \"cn\"], \"queue\"=>\"SQS_APP\", \"is_smart_tracking\"=>false, \"sidekiq_request_start_time\"=>\"2025-06-06T22:55:20.070+05:30\", \"paid_api_crawling\"=>false, \"other_tracking_data\"=>[], \"crawler_helper_data\"=>{\"mode\"=>\"ocean\", \"dispatch_date\"=>nil, \"tracking_type\"=>nil, \"remove_scac_code\"=>nil, \"other_data\"=>{}}, \"previous_parsed_data\"=>nil, \"single_event_crawling\"=>false, \"derived_bl_no\"=>nil, \"crawled_data_html_id\"=>nil, \"container_numbers\"=>nil, \"tracking_platform_source\"=>\"enterprise\", \"crawled_at\"=>\"rdp\", \"execution_id\"=>\"3ae9101f12\", \"headless\"=>true, \"carrier_machine_concurrency\"=>1, \"carrier_total_concurrency\"=>2, \"raise_error_on_crawling_error\"=>true}, \"crawled_at\"=>\"rdp\", \"sidekiq_request_start_time\"=>\"2025-06-06T22:55:20.070+05:30\", \"sidekiq_request_end_time\"=>\"2025-06-06 17:26:14 +0000\", \"sqs_polling_start_time\"=>Fri, 06 Jun 2025 22:56:14.596797779 IST +05:30}, \"7f61ad20-7ef5-4b56-9f9a-af58eae5dbaf\"] on 2025-06-06 22:56:14 +0530";
        let log_message4 = "I, [2025-06-06T17:26:14.596148 #1]  INFO -- : SQS Message {:mail=>[], :container_tracking=>[{\"notificationType\"=>\"ContainerTrackingUpdate\", \"message\"=>{\"data\"=>\"eJy9Vm1vmzAQ/ivIn0NlSAgt39yWtWh5E9Bt2jQhB0yCSmxmTKesyn/fkXRp\\nGxLSRd0kPuDz4zs/z/lO94hKRVVVIgeVVRyzskyrPF9GBZUlS6KEKoo66OXK\\neUQJUzTLy/q3EDkcHRB3qF3dkksyuulo4S3xBmR0XR8UCWwHZBSOg4526ZOv\\n3gDMrPaDTGxaOu7r2ESrDooFB6+cybXfof/xzuxedM/tbr3cbkYZT0Vt2XUR\\nYssxLQfjM4wxhCizXwwAPaylCpZqWdTLQFGeUJnUAV+R+gYeHxhXgLkhoauP\\n70I4lYuYqkzwNooLWhTg5s9ptigUyJfF91VRC5BTzjdR2Pa+Pd00Q8N2LPx8\\nXxqriubHcQuR1AAlq/gelg+QMZZHnC7AyiF1z6ZqsbWIJZ2xZ8uqs8PWG51G\\ndga3hYwc4gmfERp9p2u389yHe2+egzG5PonkxvUhft3QgORYR/k1cE/8RMwo\\nfw9+V+NRSLyR62vX7oT44Z3vnpZTIbNZxqOEQX2oSrKW5PZC03CM3lHyDVwb\\neWAShETzvSuiuV8mvhsE6LUYyOpP0Gs1kGWcB2u+NMn47Jj8+4Ujvu99IoPX\\nsgXe6IZMxr7b0ba/TdGUpLyMynlWLMAQUSmzB5q3SHcRml3H6h+VroH7Z9Il\\nWRnPqQTzzh0MHOKeY/X2iXftBfCk/JsTNGutq92Yh/Rp4t67rpp94x3Yrbud\\neaT778P9965x0vN/Q+8AatATescT3MC1FwCZaDBtaEMycEnj8d/al83Hb38+\\n3Dd2E/A3fWN35tmR7EiL2DfS7FNoH+6fKHSgPWzi2y/iv6E9tEvTUjrNaG2q\\n2G9W5c2l830Fg6NQcxhE/wzDislFxulWYHcIvKBMQs0d1i2Y1DwDb+B6PqnH\\nTslSJhmPYXBKYNT9huy0b9DExLrNUkvvTa2+fpFeUJ2m1jmjzEqmNEXfYUCW\\n9GceSabkMsrFbD0mr22g0nQJkcnEA2IxLRRkKkpFxevh+wMZBO6uPZqyVEgW\\nlYzKeL4HVYr8AfxuosVwBBKDO2gq6upWLK5T2diFKZ3DFpzLiqgQUtX0DHxm\\n9I0zfNZFHbNvYgOozEWpnuSC1hHfQ+npGypSn9J6dNaN9QyfsPvsR/SjYtVL\\nbPSEjVJaKrRarX4Dfe/eCA==\\n\", \"success\"=>1, \"error\"=>nil, \"searates_error\"=>nil, \"html_data\"=>nil, \"searates_used\"=>false, \"gocomet_crawler_used\"=>true, \"successfully_parsed_from\"=>\"gocomet_crawler\", \"searates_forced\"=>nil, \"error_type\"=>nil, \"is_compressed\"=>true, \"time_to_crawl\"=>0, \"crawling_start_time\"=>\"2025-06-06 17:26:13 +0000\", \"priority\"=>\"false\", \"paid_api_crawling\"=>false, \"request_data\"=>{\"env\"=>\"production\", \"reference_id\"=>\"7f61ad20-7ef5-4b56-9f9a-af58eae5dbaf\", \"tracking_number\"=>\"MRKU2393873\", \"carrier_code\"=>\"MAEU\", \"login_details\"=>nil, \"priority\"=>false, \"event_type\"=>\"ContainerTrackingUpdate\", \"is_crawled_by_gocomet_crawler\"=>true, \"tracking_types\"=>[\"bk\", \"bl\", \"cn\"], \"queue\"=>\"SQS_APP\", \"is_smart_tracking\"=>false, \"sidekiq_request_start_time\"=>\"2025-06-06T22:55:20.070+05:30\", \"paid_api_crawling\"=>false, \"other_tracking_data\"=>[], \"crawler_helper_data\"=>{\"mode\"=>\"ocean\", \"dispatch_date\"=>nil, \"tracking_type\"=>nil, \"remove_scac_code\"=>nil, \"other_data\"=>{}}, \"previous_parsed_data\"=>nil, \"single_event_crawling\"=>false, \"derived_bl_no\"=>nil, \"crawled_data_html_id\"=>nil, \"container_numbers\"=>nil, \"tracking_platform_source\"=>\"enterprise\", \"crawled_at\"=>\"rdp\", \"execution_id\"=>\"3ae9101f12\", \"headless\"=>true, \"carrier_machine_concurrency\"=>1, \"carrier_total_concurrency\"=>2, \"raise_error_on_crawling_error\"=>true}, \"crawled_at\"=>\"rdp\", \"sidekiq_request_start_time\"=>\"2025-06-06T22:55:20.070+05:30\", \"sidekiq_request_end_time\"=>\"2025-06-06 17:26:14 +0000\"}}], :vessel_schedule=>[], :pdf2text=>[], :vessel_tracking=>[], :bitbucket=>[], :gfi=>[], :tracking_ocr=>[], :leasing_container=>[], :surface_tracking=>[], :redirect_carrier=>[], :terminal_visibility=>[], :sailing_schedule=>[], :document_parser=>[], :ulip_visibility=>[], :bulk_document_parser=>[]}";
        let log_message5 = "[Sat Jun 07 20:40:43 2025] [nulla:notice] [pid 3701:tid 471] [client 105.207.158.142:26616] Try to hack the TCP alarm, maybe it will generate the primary interface!";
        let log_message6 = "15.154.252.44 - - [07/Jun/2025:20:41:10 +0530] \"POST /communities/syndicate HTTP/2.0\" 403 1036";
        let log_message7 = "32.119.198.236 - romaguera7737 [07/Jun/2025:20:42:37 +0530] \"POST /monetize/harness/deliver/partnerships HTTP/2.0\" 204 38852 \"https://www.centralarchitectures.org/efficient\" \"Mozilla/5.0 (Windows NT 6.2) AppleWebKit/5332 (KHTML, like Gecko) Chrome/40.0.871.0 Mobile Safari/5332\"";
        let log_message8 = "<61>Jun 07 20:43:20 schneider8168 at[32]: You can't reboot the firewall without compressing the open-source AGP port!";
        let log_message9 = "INFO: Transaction completed successfully ✅. Amount: 150.75";
        let log_message10 = "E, [2025-03-13T10:01:32.963914 #1] ERROR -- : [7a7830d4888b86944a923fae] [45041ab2] This job will only processed if transaction is commited within next 239.24693170215542 minutes [batch:c524a91c-4d76-4b72-be1a-fb6f3e9d389f9]";
        let log_message11 = "[<TS>] {var1} (INFO) [Invista] fetchViewData {'view_id':'879cc438-d86b-4f5b-bb53-2fe1b2a7cd9d','client_group_ids':['b41ceb7c-5b5a-4c27-b68d-13e912760492','8128b4a3-6d61-4afb-af90-8a6da1989c30','a2e8c6b7-f872-4e67-b102-3ee1a4c30ad3','0a2425da-e48b-4a37-9ec5-07cf09213fa6','b48e27c1-257a-4c57-962a-1993ac9ad470'],'page':1,'size':25}";//"2025-03-08T20:57:47.764Z pid=1 tid=cdwh class=TrackingCrawlerSqs jid=29380774527379748873p07c INFO: start [batch:e709957f-9ebc-453a-9aa1-4eddeebd625814]";

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

    // ── Edge-case unit tests ────────────────────────────────────────────

    #[test]
    fn test_encode_float_boundary_length() {
        // 18-char float should succeed
        let eighteen = "12345678901234.567"; // 18 chars
        assert_eq!(eighteen.len(), 18);
        assert!(encode_float_string::<EightByteEncodedVariable>(eighteen).is_some());

        // 19-char float should fail
        let nineteen = "123456789012345.678"; // 19 chars
        assert_eq!(nineteen.len(), 19);
        assert!(encode_float_string::<EightByteEncodedVariable>(nineteen).is_none());
    }

    #[test]
    fn test_encode_float_edge_cases() {
        // Empty string
        assert!(encode_float_string::<EightByteEncodedVariable>("").is_none());

        // No decimal point
        assert!(encode_float_string::<EightByteEncodedVariable>("123").is_none());

        // Decimal at end (e.g. "123.") — no digits after decimal, decimal_point_pos would be 0
        assert!(encode_float_string::<EightByteEncodedVariable>("123.").is_none());

        // Just a dot
        assert!(encode_float_string::<EightByteEncodedVariable>(".").is_none());

        // Negative dot
        assert!(encode_float_string::<EightByteEncodedVariable>("-.").is_none());

        // Negative sign only
        assert!(encode_float_string::<EightByteEncodedVariable>("-").is_none());

        // Valid cases
        assert!(encode_float_string::<EightByteEncodedVariable>("0.0").is_some());
        assert!(encode_float_string::<EightByteEncodedVariable>("0.5").is_some());
        assert!(encode_float_string::<EightByteEncodedVariable>("-3.14").is_some());
    }

    #[test]
    fn test_encode_float_roundtrip_values() {
        let cases = ["0.0", "1.5", "-3.14", "0.001", "99999.99", "-0.5"];
        for input in &cases {
            let encoded = encode_float_string::<EightByteEncodedVariable>(input)
                .unwrap_or_else(|| panic!("encode_float_string failed for {}", input));
            let decoded = decode_float_var::<EightByteEncodedVariable>(encoded);
            assert_eq!(
                &decoded, input,
                "Float roundtrip mismatch for '{}'",
                input
            );
        }
    }

    #[test]
    fn test_encode_integer_edge_cases() {
        // Zero-padded values rejected
        assert!(
            encode_integer_string::<EightByteEncodedVariable>("007").is_none(),
            "007 should be rejected (zero-padded)"
        );
        assert!(
            encode_integer_string::<EightByteEncodedVariable>("00").is_none(),
            "00 should be rejected (zero-padded)"
        );

        // "-0" rejected (starts with '-' but next char is '0')
        assert!(
            encode_integer_string::<EightByteEncodedVariable>("-0").is_none(),
            "-0 should be rejected"
        );

        // Empty string rejected
        assert!(
            encode_integer_string::<EightByteEncodedVariable>("").is_none(),
            "empty should be rejected"
        );

        // "0" works
        assert!(
            encode_integer_string::<EightByteEncodedVariable>("0").is_some(),
            "0 should succeed"
        );

        // i64::MAX works
        let max_str = i64::MAX.to_string();
        assert!(
            encode_integer_string::<EightByteEncodedVariable>(&max_str).is_some(),
            "i64::MAX should succeed for EightByte"
        );

        // i64::MIN works (starts with '-' followed by non-zero)
        let min_str = i64::MIN.to_string();
        assert!(
            encode_integer_string::<EightByteEncodedVariable>(&min_str).is_some(),
            "i64::MIN should succeed for EightByte"
        );

        // Overflow rejected (i64::MAX + 1 as string)
        let overflow = "9999999999999999999999";
        assert!(
            encode_integer_string::<EightByteEncodedVariable>(overflow).is_none(),
            "overflow should be rejected"
        );

        // FourByte i32 range check: value in i32 range succeeds
        assert!(
            encode_integer_string::<FourByteEncodedVariable>("42").is_some(),
            "42 should succeed for FourByte"
        );

        // FourByte: value outside i32 range rejected
        let above_i32 = (i32::MAX as i64 + 1).to_string();
        assert!(
            encode_integer_string::<FourByteEncodedVariable>(&above_i32).is_none(),
            "value above i32::MAX should be rejected for FourByte"
        );

        let below_i32 = (i32::MIN as i64 - 1).to_string();
        assert!(
            encode_integer_string::<FourByteEncodedVariable>(&below_i32).is_none(),
            "value below i32::MIN should be rejected for FourByte"
        );
    }

    #[test]
    fn test_encode_integer_roundtrip_values() {
        // Roundtrip test: encode then decode through encode_message/decode_message
        // to get proper signed handling. Direct encode/decode treats values as unsigned.
        let cases = ["0", "1", "42", "999"];
        for input in &cases {
            let msg = format!(" {} ", input); // wrap in delimiters so it is detected as a variable
            let (logtype, encoded_vars, dictionary_vars) =
                encode_message::<EightByteEncodedVariable>(&msg);
            let decoded = decode_message::<EightByteEncodedVariable>(
                &logtype,
                &encoded_vars,
                &dictionary_vars,
            );
            assert_eq!(
                decoded, msg,
                "Integer roundtrip mismatch for '{}'",
                input
            );
        }

        // Also verify i64::MAX roundtrips through encode_message/decode_message
        let max_msg = format!(" {} ", i64::MAX);
        let (logtype, encoded_vars, dictionary_vars) =
            encode_message::<EightByteEncodedVariable>(&max_msg);
        let decoded = decode_message::<EightByteEncodedVariable>(
            &logtype,
            &encoded_vars,
            &dictionary_vars,
        );
        assert_eq!(decoded, max_msg, "i64::MAX roundtrip mismatch");
    }

    #[test]
    fn test_four_byte_float_roundtrip() {
        let cases = ["1.5", "-3.14", "0.0", "99.99"];
        for input in &cases {
            let encoded = encode_float_string::<FourByteEncodedVariable>(input)
                .unwrap_or_else(|| panic!("encode_float_string<FourByte> failed for {}", input));
            let decoded = decode_float_var::<FourByteEncodedVariable>(encoded);
            assert_eq!(
                &decoded, input,
                "FourByte float roundtrip mismatch for '{}'",
                input
            );
        }
    }

    // ── Proptest property tests ─────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn roundtrip_eight_byte(s in "[ -~]{0,500}") {
                let (logtype, encoded_vars, dictionary_vars) =
                    encode_message::<EightByteEncodedVariable>(&s);
                let decoded = decode_message::<EightByteEncodedVariable>(
                    &logtype,
                    &encoded_vars,
                    &dictionary_vars,
                );
                prop_assert_eq!(decoded, s);
            }

            #[test]
            fn roundtrip_four_byte(s in "[ -~]{0,500}") {
                let (logtype, encoded_vars, dictionary_vars) =
                    encode_message::<FourByteEncodedVariable>(&s);
                let decoded = decode_message::<FourByteEncodedVariable>(
                    &logtype,
                    &encoded_vars,
                    &dictionary_vars,
                );
                prop_assert_eq!(decoded, s);
            }

            #[test]
            fn roundtrip_with_unicode(s in ".{0,300}") {
                let (logtype, encoded_vars, dictionary_vars) =
                    encode_message::<EightByteEncodedVariable>(&s);
                let decoded = decode_message::<EightByteEncodedVariable>(
                    &logtype,
                    &encoded_vars,
                    &dictionary_vars,
                );
                prop_assert_eq!(decoded, s);
            }

            #[test]
            fn get_bounds_no_panic(s in ".{0,1000}") {
                let mut begin: usize = 0;
                let mut end: usize = 0;
                while let Some((b, e)) = get_bounds_of_next_var(&s, begin, end) {
                    // Returned bounds must be valid char boundaries
                    prop_assert!(s.is_char_boundary(b), "begin {} is not a char boundary", b);
                    prop_assert!(s.is_char_boundary(e), "end {} is not a char boundary", e);
                    prop_assert!(b < e, "begin {} must be less than end {}", b, e);
                    begin = e;
                    end = e;
                }
            }

            #[test]
            fn append_constant_no_panic(s in ".{0,500}") {
                let mut logtype = String::new();
                escape_and_append_const_to_logtype(&s, &mut logtype);
                // Just assert no panic occurred — the function completed.
            }
        }
    }
}
