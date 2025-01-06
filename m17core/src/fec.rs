use crate::bits::{Bits, BitsMut};
use log::debug;

struct Transition {
    input: u8,
    output: [u8; 2],
    source: usize,
}

static TRANSITIONS: [Transition; 32] = [
    Transition {
        input: 0,
        output: [0, 0],
        source: 0,
    },
    Transition {
        input: 0,
        output: [1, 1],
        source: 1,
    },
    Transition {
        input: 0,
        output: [1, 0],
        source: 2,
    },
    Transition {
        input: 0,
        output: [0, 1],
        source: 3,
    },
    Transition {
        input: 0,
        output: [0, 1],
        source: 4,
    },
    Transition {
        input: 0,
        output: [1, 0],
        source: 5,
    },
    Transition {
        input: 0,
        output: [1, 1],
        source: 6,
    },
    Transition {
        input: 0,
        output: [0, 0],
        source: 7,
    },
    Transition {
        input: 0,
        output: [0, 1],
        source: 8,
    },
    Transition {
        input: 0,
        output: [1, 0],
        source: 9,
    },
    Transition {
        input: 0,
        output: [1, 1],
        source: 10,
    },
    Transition {
        input: 0,
        output: [0, 0],
        source: 11,
    },
    Transition {
        input: 0,
        output: [0, 0],
        source: 12,
    },
    Transition {
        input: 0,
        output: [1, 1],
        source: 13,
    },
    Transition {
        input: 0,
        output: [1, 0],
        source: 14,
    },
    Transition {
        input: 0,
        output: [0, 1],
        source: 15,
    },
    Transition {
        input: 1,
        output: [1, 1],
        source: 0,
    },
    Transition {
        input: 1,
        output: [0, 0],
        source: 1,
    },
    Transition {
        input: 1,
        output: [0, 1],
        source: 2,
    },
    Transition {
        input: 1,
        output: [1, 0],
        source: 3,
    },
    Transition {
        input: 1,
        output: [1, 0],
        source: 4,
    },
    Transition {
        input: 1,
        output: [0, 1],
        source: 5,
    },
    Transition {
        input: 1,
        output: [0, 0],
        source: 6,
    },
    Transition {
        input: 1,
        output: [1, 1],
        source: 7,
    },
    Transition {
        input: 1,
        output: [1, 0],
        source: 8,
    },
    Transition {
        input: 1,
        output: [0, 1],
        source: 9,
    },
    Transition {
        input: 1,
        output: [0, 0],
        source: 10,
    },
    Transition {
        input: 1,
        output: [1, 1],
        source: 11,
    },
    Transition {
        input: 1,
        output: [1, 1],
        source: 12,
    },
    Transition {
        input: 1,
        output: [0, 0],
        source: 13,
    },
    Transition {
        input: 1,
        output: [0, 1],
        source: 14,
    },
    Transition {
        input: 1,
        output: [1, 0],
        source: 15,
    },
];

pub(crate) fn p_1(step: usize) -> (bool, bool) {
    let mod61 = step % 61;
    let is_even = mod61 % 2 == 0;
    (mod61 > 30 || is_even, mod61 < 30 || is_even)
}

pub(crate) fn p_2(step: usize) -> (bool, bool) {
    let mod6 = step % 6;
    (true, mod6 != 5)
}

pub(crate) fn p_3(step: usize) -> (bool, bool) {
    let mod4 = step % 4;
    (true, mod4 != 3)
}

fn best_previous(table: &[[u8; 32]; 244], step: usize, state: usize) -> u8 {
    if step == 0 {
        if state == 0 {
            return 0;
        } else {
            return u8::MAX;
        }
    }
    let prev1 = table[step - 1][state * 2];
    let prev2 = table[step - 1][state * 2 + 1];
    prev1.min(prev2)
}

fn hamming_distance(first: &[u8], second: &[u8]) -> u8 {
    first
        .iter()
        .zip(second.iter())
        .map(|(x, y)| if *x == *y { 0 } else { 1 })
        .sum()
}

// maximum 368 type 3 bits, maximum 240 type 1 bits, 4 flush bits
pub(crate) fn decode(
    type3: &[u8], // up to len 46
    input_len: usize,
    puncture: fn(usize) -> (bool, bool),
) -> Option<[u8; 30]> {
    let type3_bits = Bits::new(type3);
    let mut type3_iter = type3_bits.iter();
    let mut table = [[0u8; 32]; 244];
    for step in 0..(input_len + 4) {
        let (use_g1, use_g2) = puncture(step);
        let split_idx = if use_g1 && use_g2 { 2 } else { 1 };
        let mut input_bits = [0u8; 2];
        input_bits[0] = type3_iter.next().unwrap();
        let step_input = if split_idx == 1 {
            &input_bits[0..1]
        } else {
            input_bits[1] = type3_iter.next().unwrap();
            &input_bits[0..2]
        };
        for (t_idx, t) in TRANSITIONS.iter().enumerate() {
            let t_offer = if use_g1 && use_g2 {
                &t.output[..]
            } else if use_g1 {
                &t.output[0..1]
            } else {
                &t.output[1..2]
            };
            let step_dist = hamming_distance(step_input, t_offer);
            table[step][t_idx] = best_previous(&table, step, t.source).saturating_add(step_dist);
        }
    }
    let (mut best_idx, best) = table[input_len + 3]
        .iter()
        .enumerate()
        .min_by_key(|(_, i)| *i)
        .unwrap();
    debug!("Best score is {best}, transition {best_idx}");
    if *best > 6 {
        None
    } else {
        let mut out = [0u8; 30];
        let mut out_bits = BitsMut::new(&mut out);
        for step in (0..(input_len + 4)).rev() {
            let input = TRANSITIONS[best_idx].input;
            if step < input_len {
                out_bits.set_bit(step, input);
            }
            if step > 0 {
                let state = TRANSITIONS[best_idx].source;
                let prev1 = table[step - 1][state * 2];
                let prev2 = table[step - 1][state * 2 + 1];
                best_idx = if prev1 < prev2 {
                    state * 2
                } else {
                    state * 2 + 1
                };
            }
        }
        Some(out)
    }
}
