//! Signal trace machinery: turns the signal list in `signals.rs` into text lines,
//! and queues them for the serial port.
//!
//! You shouldn't need to edit this file to add signals: see `signals.rs`.
//!
//! # Wire format (USART2 -> ST-LINK USB serial, 230400 baud, 8N1)
//! ```text
//! #F,10,t_ms[ms],joy_x[raw],joy_y[raw],...     header: tag, period ms, columns
//! F,123450,2051,2047,...                        data:   tag, time, values
//! ```
//! - One line per sample, `\n` terminated, plain ASCII: readable in any serial terminal.
//! - `t_ms` = milliseconds since power-on (counted by the 10 ms loop).
//! - Headers are sent at power-on and every 10 s, so a recorder that connects
//!   late still learns the columns.
//! - Arrays become numbered columns: `moist: [u8; 6]` -> `moist0 .. moist5`.
//! - Values are integers in the stated unit. bool = 0/1. `enum` = a code (tables
//!   in `signals.rs`). 255 in an `idx` column = none.

/// Longest line any group may produce (checked by a unit test in signals.rs).
pub const LINE_MAX: usize = 1024;

/// How often the headers are repeated.
pub const HEADER_EVERY_MS: u64 = 10_000;

/// One entry of the signal list.
#[derive(Clone, Copy, Debug)]
pub struct Signal {
    pub name: &'static str,
    pub unit: &'static str,
    /// Number of columns: 1, or the array length
    pub count: usize,
}

/// Declares a group of signals sampled together, see `signals.rs`. Generates:
///   - a struct with one field per signal (the firmware must fill every one),
///   - `header()`: the header line (names + units),
///   - `frame()`: the data line, always in the same order as the header.
macro_rules! signal_group {
    (
        $(#[$meta:meta])*
        pub struct $name:ident, tag = $tag:literal, period_ms = $period:literal {
            $( $field:ident : $ty:ty, $unit:literal; )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct $name {
            $( pub $field: $ty, )*
        }

        impl $name {
            /// First character of every line of this group
            pub const TAG: u8 = $tag;
            /// Sample period
            pub const PERIOD_MS: u32 = $period;
            /// Names, units and column counts, in line order
            pub const SIGNALS: &'static [$crate::trace::Signal] = &[
                $( $crate::trace::Signal {
                    name: stringify!($field),
                    unit: $unit,
                    count: <$ty as $crate::trace::TraceValue>::COUNT,
                }, )*
            ];
            /// Longest possible data line: tag, time, values, newline
            pub const MAX_LINE: usize = 2
                + <u64 as $crate::trace::TraceValue>::MAX_LEN
                $( + 1 + <$ty as $crate::trace::TraceValue>::MAX_LEN )*
                + 1;

            /// Header line: `#<tag>,<period>,t_ms[ms],name[unit],...`
            pub fn header<const N: usize>(line: &mut $crate::trace::Line<N>) {
                $crate::trace::header_line(line, Self::TAG, Self::PERIOD_MS, Self::SIGNALS);
            }

            /// Data line: `<tag>,<t_ms>,value,...`
            pub fn frame<const N: usize>(&self, t_ms: u64, line: &mut $crate::trace::Line<N>) {
                use $crate::trace::TraceValue;
                line.clear();
                line.push(Self::TAG);
                line.push(b',');
                t_ms.write_to(line);
                $(
                    line.push(b',');
                    self.$field.write_to(line);
                )*
                line.push(b'\n');
            }
        }
    };
}
pub(crate) use signal_group;

pub fn header_line<const N: usize>(line: &mut Line<N>, tag: u8, period_ms: u32, signals: &[Signal]) {
    line.clear();
    line.push(b'#');
    line.push(tag);
    line.push(b',');
    line.push_u64(period_ms as u64);
    line.push_str(",t_ms[ms]");
    for s in signals {
        for i in 0..s.count {
            line.push(b',');
            line.push_str(s.name);
            if s.count > 1 {
                line.push_u64(i as u64);
            }
            line.push(b'[');
            line.push_str(s.unit);
            line.push(b']');
        }
    }
    line.push(b'\n');
}

/// A value that can be written into a trace line.
pub trait TraceValue {
    /// Columns this value takes
    const COUNT: usize = 1;
    /// Longest text it can produce
    const MAX_LEN: usize;
    fn write_to<const N: usize>(&self, line: &mut Line<N>);
}

macro_rules! unsigned_value {
    ($($t:ty => $len:expr),*) => {$(
        impl TraceValue for $t {
            const MAX_LEN: usize = $len;
            fn write_to<const N: usize>(&self, line: &mut Line<N>) {
                line.push_u64(*self as u64);
            }
        }
    )*};
}
unsigned_value!(u8 => 3, u16 => 5, u32 => 10, u64 => 20);

impl TraceValue for bool {
    const MAX_LEN: usize = 1;
    fn write_to<const N: usize>(&self, line: &mut Line<N>) {
        line.push(if *self { b'1' } else { b'0' });
    }
}

impl<T: TraceValue, const L: usize> TraceValue for [T; L] {
    const COUNT: usize = L * T::COUNT;
    const MAX_LEN: usize = L * (T::MAX_LEN + 1) - 1;
    fn write_to<const N: usize>(&self, line: &mut Line<N>) {
        for (i, v) in self.iter().enumerate() {
            if i > 0 {
                line.push(b',');
            }
            v.write_to(line);
        }
    }
}

/// Fixed-size text line, no allocator. If it overflows, `ok()` is false and the
/// line must not be sent (a cut-off line would shift every column after it).
pub struct Line<const N: usize> {
    buf: [u8; N],
    len: usize,
    overflow: bool,
}

impl<const N: usize> Line<N> {
    pub const fn new() -> Self {
        Line { buf: [0; N], len: 0, overflow: false }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.overflow = false;
    }

    pub fn push(&mut self, b: u8) {
        if self.len < N {
            self.buf[self.len] = b;
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }

    pub fn push_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.push(b);
        }
    }

    /// Decimal digits, without pulling in the formatting machinery
    pub fn push_u64(&mut self, mut v: u64) {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for &d in &digits[i..] {
            self.push(d);
        }
    }

    pub fn ok(&self) -> bool {
        !self.overflow
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte FIFO between the main loop (adds whole lines) and the serial
/// interrupt (sends one byte at a time). Fixed size, no allocator.
pub struct ByteQueue<const N: usize> {
    buf: [u8; N],
    head: usize, // next byte to send
    len: usize,  // bytes waiting
}

impl<const N: usize> ByteQueue<N> {
    pub const fn new() -> Self {
        ByteQueue { buf: [0; N], head: 0, len: 0 }
    }

    /// Queue all of `bytes`, or nothing if they don't fit (so a line is never
    /// half-sent). Returns whether they were queued.
    pub fn push_all(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > N - self.len {
            return false;
        }
        for &b in bytes {
            self.buf[(self.head + self.len) % N] = b;
            self.len += 1;
        }
        true
    }

    pub fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(b)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for ByteQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// 255 for "none" in `idx` columns
pub fn idx_code(idx: Option<usize>) -> u8 {
    idx.map_or(255, |i| i.min(254) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text<const N: usize>(line: &Line<N>) -> &str {
        core::str::from_utf8(line.as_bytes()).unwrap()
    }

    #[test]
    fn numbers_are_written_in_decimal() {
        let mut l = Line::<32>::new();
        for (v, s) in [(0u64, "0"), (7, "7"), (10, "10"), (4095, "4095"), (u64::MAX, "18446744073709551615")] {
            l.clear();
            l.push_u64(v);
            assert_eq!(text(&l), s);
        }
    }

    #[test]
    fn max_len_matches_the_longest_number() {
        let mut l = Line::<32>::new();
        for (v, len) in [(u8::MAX as u64, u8::MAX_LEN), (u16::MAX as u64, u16::MAX_LEN), (u32::MAX as u64, u32::MAX_LEN), (u64::MAX, u64::MAX_LEN)] {
            l.clear();
            l.push_u64(v);
            assert_eq!(l.as_bytes().len(), len);
        }
    }

    #[test]
    fn line_overflow_is_reported() {
        let mut l = Line::<4>::new();
        l.push_str("abcd");
        assert!(l.ok());
        l.push(b'e');
        assert!(!l.ok());
        assert_eq!(text(&l), "abcd");
        l.clear();
        assert!(l.ok());
    }

    #[test]
    fn arrays_are_comma_separated() {
        let mut l = Line::<32>::new();
        [true, false, true].write_to(&mut l);
        assert_eq!(text(&l), "1,0,1");
        assert_eq!(<[bool; 3]>::COUNT, 3);
        assert_eq!(<[bool; 3]>::MAX_LEN, 5);
    }

    #[test]
    fn queue_is_first_in_first_out() {
        let mut q = ByteQueue::<8>::new();
        assert!(q.push_all(b"abc"));
        assert_eq!(q.pop(), Some(b'a'));
        assert_eq!(q.pop(), Some(b'b'));
        assert_eq!(q.pop(), Some(b'c'));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn queue_never_takes_half_a_line() {
        let mut q = ByteQueue::<8>::new();
        assert!(q.push_all(b"12345"));
        assert!(!q.push_all(b"6789")); // only 3 free
        assert_eq!(q.len(), 5);
        assert!(q.push_all(b"678")); // exactly fills it
        assert!(!q.push_all(b"9"));
    }

    #[test]
    fn queue_wraps_around() {
        let mut q = ByteQueue::<4>::new();
        for round in 0..10u8 {
            assert!(q.push_all(&[round, round + 1, round + 2]));
            assert_eq!(q.pop(), Some(round));
            assert_eq!(q.pop(), Some(round + 1));
            assert_eq!(q.pop(), Some(round + 2));
            assert!(q.is_empty());
        }
    }

    #[test]
    fn idx_none_is_255() {
        assert_eq!(idx_code(None), 255);
        assert_eq!(idx_code(Some(3)), 3);
    }
}
