extern crate core;

use std::collections::HashSet;
use std::hash::Hash;

pub use deunicode;
pub use rayon;
pub use smallvec;
pub use ustr;

pub mod coordinates;
mod graph;
pub mod location;
pub mod locations_db;
pub mod search;

const SCORE_SOFT_MAX: i64 = 1000;
const STATE_CODE_BOOST: i64 = 32;
const SUBDIV_CODE_BOOST: i64 = 16;
const LEV_3_LENGTH_MAX: usize = 10;
const LEV_2_LENGTH_MAX: usize = 20;
const LEV_LENGTH_MAX: usize = 40;

const SINGLE_WORD_MATCH_PENALTY: i64 = 100;

const SEARCH_INCLUSION_THRESHOLD: i64 = 400;
const GRAPH_EDGE_THRESHOLD: i64 = 600;

pub fn normalize(s: &str) -> String {
    deunicode::deunicode(s).to_lowercase()
}

pub fn dedup<T: Eq + Hash>(vec: Vec<T>) -> Vec<T> {
    vec.into_iter()
        .collect::<HashSet<T>>()
        .into_iter()
        .collect()
}
