//! How a value is laid out in linear memory.
//!
//! One shape for everything that does not fit in a slot, because the
//! alternative is a rule per type and a backend that has to remember which
//! one it is looking at.
//!
//! Every field, element and tag is eight bytes. That wastes space on a list
//! of booleans and it means nothing has to be told how wide anything is,
//! which is the trade a first backend should make in that direction: the
//! measurement that would justify packing does not exist yet, and
//! `design/01-principles.md` is fairly clear about what a machine built
//! without one is worth.
//!
//! Layouts:
//!
//! - an aggregate is `[tag][field 0][field 1]...`, with the tag left out when
//!   the layout has one variant, since there is nothing to tell apart
//! - a list is `[length][element 0][element 1]...`
//! - a string is `[length in bytes][the bytes, padded to eight]`
//!
//! Allocation is a bump pointer living at address 0, so the module needs no
//! global section and no import. Nothing is ever freed. That is not a
//! garbage collector's absence being ignored: values in this language are
//! immutable and a compiled `test` block runs once, so the first program
//! that outlives its memory is the one that motivates writing one, and it
//! will have a number attached.

/// Where the bump pointer lives.
pub const BUMP: u32 = 0;

/// Where the innermost handler frame's address lives, or zero when no
/// handler is installed.
///
/// The one piece of state a compiled program keeps that the source does not
/// name. A frame is `[next][effect][state][code 0][code 1]...`: the frame
/// under it, which effect it answers for, the address of its state, and a
/// table index per operation. `with` links a frame in and unlinks it when
/// the block ends, and performing walks from here down until the effect
/// matches. See `design/05-backend.md`.
pub const HANDLERS: u32 = 8;

/// Where allocation starts, leaving room for the two words above.
pub const HEAP_START: u32 = 16;

/// The width of every field, element and tag.
pub const WORD: u32 = 8;

/// How many bytes a handler frame with this many operations takes.
pub fn frame_size(operations: usize) -> u32 {
    (3 + operations as u32) * WORD
}

/// Where an operation's code pointer sits inside a handler frame.
pub fn operation_offset(operation: usize) -> u32 {
    (3 + operation as u32) * WORD
}

/// How many bytes an aggregate of this shape takes.
pub fn aggregate_size(tagged: bool, fields: usize) -> u32 {
    (if tagged { 1 } else { 0 } + fields as u32) * WORD
}

/// Where a field sits inside an aggregate.
pub fn field_offset(tagged: bool, field: usize) -> u32 {
    (if tagged { 1 } else { 0 } + field as u32) * WORD
}

/// How many bytes a list of this many elements takes.
pub fn list_size(elements: usize) -> u32 {
    WORD + elements as u32 * WORD
}

/// Where an element sits inside a list, by position.
pub fn element_offset(index: usize) -> u32 {
    WORD + index as u32 * WORD
}

/// How many bytes a string of this many bytes takes, rounded up so that
/// whatever is allocated next still starts on a word.
pub fn string_size(bytes: usize) -> u32 {
    WORD + (bytes as u32).div_ceil(WORD) * WORD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_has_no_tag_and_a_choice_does() {
        assert_eq!(aggregate_size(false, 2), 16);
        assert_eq!(aggregate_size(true, 2), 24);
        assert_eq!(field_offset(false, 0), 0);
        assert_eq!(field_offset(true, 0), 8);
    }

    #[test]
    fn a_list_puts_its_length_first() {
        assert_eq!(list_size(0), 8);
        assert_eq!(list_size(3), 32);
        assert_eq!(element_offset(0), 8);
        assert_eq!(element_offset(2), 24);
    }

    /// The one that is easy to get wrong: a string whose bytes do not fill a
    /// word still has to leave the next allocation on one.
    #[test]
    fn a_string_rounds_up_to_a_word() {
        assert_eq!(string_size(0), 8);
        assert_eq!(string_size(1), 16);
        assert_eq!(string_size(8), 16);
        assert_eq!(string_size(9), 24);
    }
}
