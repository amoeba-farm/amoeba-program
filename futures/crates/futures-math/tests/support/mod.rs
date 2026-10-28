//! TEST ONLY: shared by the tests and `benches/solve.rs` (which includes this
//! folder by path): the pre-speed-up solver as a reference and the books.
#![allow(dead_code)]

pub mod books;
pub mod reference;

use ameba_futures_math::{Arc, Pos, Solution, Taker};

/// `solve_with_marks`, new or reference.
pub type Solver = fn(&[Arc], Option<&Taker>, Pos, &mut dyn FnMut(&'static str, &[u64])) -> Solution;
