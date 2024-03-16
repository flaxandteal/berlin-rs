use std::cmp::Ordering;

use schemars::JsonSchema;
use serde::Serialize;
use strsim::normalized_levenshtein as similarity_algo;
use unicode_segmentation::UnicodeSegmentation;
use ustr::{Ustr, UstrSet};

use crate::LEV_LENGTH_MAX;
use crate::SCORE_SOFT_MAX;

const STOP_WORDS: [&str; 15] = [
    "any", "all", "are", "is", "at", "to", "in", "on", "of", "for", "by", "and", "was", "did",
    "the",
];

#[derive(Debug)]
pub struct SearchTerm {
    pub raw: String,
    pub normalized: String,
    pub codes: Vec<MatchDef<Ustr>>,
    pub matches: SearchableStringSet,
    pub state_filter: Option<Ustr>,
    pub limit: usize,
    pub lev_dist: u32,
}

#[derive(Debug)]
pub struct SearchableStringSet {
    pub stop_words: Vec<Ustr>,
    exact: Vec<MatchDef<Ustr>>,
    not_exact: Vec<MatchDef<String>>,
}

impl SearchTerm {
    pub fn add_code(&mut self, u: Ustr) {
        let str = u.as_str();
        let start = self.normalized.find(str).unwrap();
        self.codes.push(MatchDef {
            term: u,
            offset: Offset {
                start,
                end: start + str.len(),
            },
        })
    }
}

#[derive(Debug)]
pub struct MatchDef<T> {
    pub term: T,
    pub offset: Offset,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, JsonSchema, Serialize)]
pub struct Offset {
    pub start: usize,
    pub end: usize,
}

impl PartialOrd for Offset {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.start.partial_cmp(&other.start)
    }
}

impl Ord for Offset {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.start.cmp(&other.start) {
            Ordering::Equal => self.end.cmp(&other.end),
            ord => ord,
        }
    }
}

#[derive(PartialEq, Eq, Copy, Clone, JsonSchema, Serialize)]
pub struct Score {
    pub score: i64,
    pub offset: Offset,
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.score.cmp(&other.score))
    }
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.partial_cmp(other) {
            None => Ordering::Equal,
            Some(ord) => match ord {
                Ordering::Equal => self.offset.cmp(&other.offset),
                ord => ord,
            },
        }
    }
}

impl SearchableStringSet {
    pub fn new(stop_words: Vec<Ustr>) -> SearchableStringSet {
        SearchableStringSet {
            stop_words: stop_words,
            exact: vec![],
            not_exact: vec![],
        }
    }
    pub fn match_str(&self, subject: &str) -> Option<Score> {
        let exact = self
            .exact
            .iter()
            .filter_map(|m| match m.term == subject {
                true => Some(Score {
                    score: SCORE_SOFT_MAX + m.term.len() as i64,
                    offset: m.offset,
                }),
                false => None,
            })
            .max();
        match exact {
            Some(s) => Some(s),
            None => self
                .not_exact
                .iter()
                .map(|w| {
                    let score = if w.term.len() > 3 && subject.starts_with(&w.term) {
                        SCORE_SOFT_MAX + (2 * w.term.len() as i64)
                    } else {
                        match w.term.len() > subject.len() - 2 && w.term.len() < subject.len() + 2 {
                            true => {
                                (similarity_algo(subject, &w.term) * SCORE_SOFT_MAX as f64) as i64
                            }
                            false => 0,
                        }
                    };
                    Score {
                        score,
                        offset: w.offset,
                    }
                })
                .max(),
        }
    }
    pub fn build_search<'c>(
        &'c self,
        mut op: fst::map::OpBuilder<'c>,
        mut search_action: impl FnMut(fst::map::OpBuilder<'c>, &'c str) -> fst::map::OpBuilder<'c>,
        mut grab_action: impl FnMut(&'c Ustr) -> Option<&UstrSet>,
    ) -> (fst::map::OpBuilder, UstrSet) {
        let mut pre_filtered: UstrSet = UstrSet::default();
        let ungrabbed = {
            self.exact
                .iter()
                .filter_map(|t| match grab_action(&t.term) {
                    Some(locs) => {
                        pre_filtered.extend(locs);
                        None
                    }
                    _ => Some(t.term.as_str()),
                })
        };
        op = self
            .not_exact
            .iter()
            .map(|ne| ne.term.as_str())
            .chain(ungrabbed)
            .fold(op, |op, t| search_action(op, t));
        (op, pre_filtered)
    }

    pub fn add(&mut self, matchable: &str, normalized: &String, allow_inexact: bool) {
        // TODO: do we really want to add inexact matches of <2 chars?
        match Ustr::from_existing(matchable) {
            Some(u) => match matchable.len() {
                0 | 1 => {}                             // ignore
                _ if self.stop_words.contains(&u) => {} // ignore stop words
                _ => self.add_exact(u, normalized),
            },
            None if allow_inexact && matchable.chars().count() < LEV_LENGTH_MAX => {
                self.add_not_exact(matchable.to_string(), normalized)
            }
            None => {}
        }
    }
    fn add_exact(&mut self, u: Ustr, normalized: &String) {
        let str = u.as_str();
        let start = normalized.find(str).unwrap();
        self.exact.push(MatchDef {
            term: u,
            offset: Offset {
                start,
                end: start + str.len(),
            },
        })
    }
    fn add_not_exact(&mut self, ne: String, normalized: &String) {
        let start = normalized.find(&ne).unwrap();
        self.not_exact.push(MatchDef {
            offset: Offset {
                start,
                end: start + ne.len(),
            },
            term: ne,
        })
    }
}

impl SearchTerm {
    pub fn from_raw_query(
        raw: String,
        state_filter: Option<String>,
        limit: usize,
        lev_dist: u32,
    ) -> Self {
        let normalized = crate::normalize(&raw);
        let split_words: Vec<&str> = normalized.unicode_words().collect();
        let stop_words: Vec<Ustr> = split_words
            .iter()
            .filter_map(|w| Ustr::from_existing(w).filter(|w| STOP_WORDS.contains(&w.as_str())))
            .collect();
        let mut st = SearchTerm {
            raw,
            normalized: normalized.clone(),
            state_filter: state_filter.and_then(|s| Ustr::from_existing(&s)),
            lev_dist,
            limit,
            codes: vec![],
            matches: SearchableStringSet::new(stop_words.clone()),
        };
        // info!("Split words: {:?}", split_words);
        for (i, w) in split_words.iter().enumerate() {
            if split_words.len() > i + 1 {
                let doublet: String = [w, split_words[i + 1]].join(" ");
                st.matches.add(&doublet, &st.normalized, true);
                if split_words.len() > i + 2 {
                    let triplet = [&doublet, split_words[i + 2]].join(" ");
                    st.matches.add(&triplet, &st.normalized, false);
                }
            }
            st.matches.add(w, &st.normalized, true)
        }
        st
    }
    pub fn codes_match(&self, subject_codes: &[Ustr], score: i64) -> Option<Score> {
        let res: Option<Score> = subject_codes
            .iter()
            .flat_map(|c| {
                self.codes
                    .iter()
                    .filter(|tc| tc.term == *c)
                    .map(|tc| Score {
                        offset: tc.offset,
                        score,
                    })
            })
            .max();
        res
    }
    pub fn match_str(&self, subject: &str) -> Option<Score> {
        self.matches.match_str(subject)
    }
    pub fn build_search<'c>(
        &'c self,
        op: fst::map::OpBuilder<'c>,
        search_action: impl FnMut(fst::map::OpBuilder<'c>, &'c str) -> fst::map::OpBuilder<'c>,
        grab_action: impl FnMut(&'c Ustr) -> Option<&UstrSet>,
    ) -> (fst::map::OpBuilder, UstrSet) {
        self.matches.build_search(op, search_action, grab_action)
    }
}
