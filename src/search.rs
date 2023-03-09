use std::cmp::Ordering;

use schemars::JsonSchema;
use serde::Serialize;
use strsim::normalized_levenshtein as similarity_algo;
use unicode_segmentation::UnicodeSegmentation;
use ustr::Ustr;

use crate::SCORE_SOFT_MAX;

const STOP_WORDS: [&str; 11] = [
    "at", "to", "in", "on", "of", "for", "by", "and", "was", "did", "the",
];

#[derive(Debug)]
pub struct SearchTerm {
    pub raw: String,
    pub normalized: String,
    pub stop_words: Vec<Ustr>,
    pub codes: Vec<MatchDef<Ustr>>,
    pub exact_matches: Vec<MatchDef<Ustr>>,
    pub not_exact_matches: Vec<MatchDef<String>>,
    pub state_filter: Option<Ustr>,
    pub limit: usize,
    pub lev_dist: u32,
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
    pub fn add_exact(&mut self, u: Ustr) {
        let str = u.as_str();
        let start = self.normalized.find(str).unwrap();
        self.exact_matches.push(MatchDef {
            term: u,
            offset: Offset {
                start,
                end: start + str.len(),
            },
        })
    }
    pub fn add_not_exact(&mut self, ne: String) {
        let start = self.normalized.find(&ne).unwrap();
        self.not_exact_matches.push(MatchDef {
            offset: Offset {
                start,
                end: start + ne.len(),
            },
            term: ne,
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
            stop_words: stop_words.clone(),
            codes: vec![],
            exact_matches: vec![],
            not_exact_matches: vec![],
        };
        // info!("Split words: {:?}", split_words);
        for (i, w) in split_words.iter().enumerate() {
            if split_words.len() > i + 1 {
                let doublet: String = [w, split_words[i + 1]].join(" ");
                match Ustr::from_existing(&doublet) {
                    Some(u) => st.add_exact(u),
                    None => st.add_not_exact(doublet.clone()),
                }
                if split_words.len() > i + 2 {
                    let triplet = [&doublet, split_words[i + 2]].join(" ");
                    if let Some(u) = Ustr::from_existing(&triplet) {
                        st.add_exact(u)
                    }
                }
            }
            match Ustr::from_existing(w) {
                None => st.add_not_exact(w.to_string()),
                Some(known_ustr) => match w.len() {
                    0 | 1 => {}                                 // ignore
                    _ if stop_words.contains(&known_ustr) => {} // ignore stop words
                    2 | 3 => {
                        st.add_code(known_ustr);
                        st.add_exact(known_ustr)
                    }
                    _ => st.add_exact(known_ustr),
                },
            }
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
        let exact = self
            .exact_matches
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
                .not_exact_matches
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
}
