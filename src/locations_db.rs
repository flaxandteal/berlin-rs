use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Instant;

use csv::ReaderBuilder;
use fst::{Automaton, Streamer};
use rayon::iter::{
    IntoParallelIterator, IntoParallelRefIterator, ParallelBridge, ParallelIterator,
};
use serde_json::Value;
use tracing::{debug, error, info};
use ustr::{Ustr, UstrMap, UstrSet};

use crate::graph::ResultsGraph;
use crate::location::{AnyLocation, CsvLocode, LocData, Location};
use crate::search::{Score, SearchTerm};
use crate::SEARCH_INCLUSION_THRESHOLD;

#[derive(Default)]
pub struct LocationsDb {
    pub all: UstrMap<Location>,
    // state names by code
    pub state_by_code: UstrMap<Ustr>,
    // key is in format "gb:lon", value is name
    pub subdiv_by_code: UstrMap<Ustr>,
    pub by_word_map: UstrMap<UstrSet>,
    pub by_word_vec: Vec<(Ustr, UstrSet)>,
    pub fst: fst::Map<Vec<u8>>,
}

impl LocationsDb {
    pub fn insert(&mut self, l: Location) {
        match &l.data {
            LocData::St(s) => {
                self.state_by_code.insert(s.alpha2, s.name);
            }
            LocData::Subdv(sd) => {
                self.subdiv_by_code.insert(l.id, sd.name);
            }
            LocData::Locd(_) => {}
            LocData::Airp(_) => {}
        }
        self.all.insert(l.key, l);
    }
    pub fn mk_fst(self) -> Self {
        let mut words_map: UstrMap<UstrSet> = UstrMap::default();
        self.all.iter().for_each(|(key, loc)| {
            let codes = loc.get_codes();
            let names = loc.get_names();
            let words_iter = loc.words.iter().chain(codes.iter()).chain(names.iter());
            words_iter.for_each(|w| {
                let old = match words_map.get_mut(w) {
                    None => {
                        let new = UstrSet::default();
                        words_map.insert(*w, new);
                        words_map.get_mut(w).unwrap()
                    }
                    Some(set) => set,
                };
                old.insert(*key);
            })
        });
        let mut words_vec = words_map
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect::<Vec<_>>();
        words_vec.sort_unstable_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let fst = fst::Map::from_iter(
            words_vec
                .iter()
                .enumerate()
                .map(|(i, (word, _))| (word.as_str(), i as u64)),
        )
        .expect("Build FST");
        LocationsDb {
            all: self.all,
            state_by_code: self.state_by_code,
            subdiv_by_code: self.subdiv_by_code,
            by_word_map: words_map,
            by_word_vec: words_vec,
            fst,
        }
    }
    pub fn search(&self, st: &SearchTerm) -> Vec<(Ustr, Score)> {
        let mut pre_filtered: UstrSet = UstrSet::default();
        st.exact_matches.iter().for_each(|term| {
            if let Some(locs) = self.by_word_map.get(&term.term) {
                pre_filtered.extend(locs);
            };
        });
        let not_exact = st.not_exact_matches.iter().map(|ne| ne.term.as_str());
        let mut stream = not_exact
            .fold(fst::map::OpBuilder::new(), |op, term| {
                match term.len() > 3 {
                    true => {
                        let prefix_matcher = fst::automaton::Str::new(term).starts_with();
                        let autom = fst::automaton::Levenshtein::new(term, st.lev_dist)
                            .expect("build automaton")
                            .union(prefix_matcher);
                        op.add(self.fst.search(autom))
                    }
                    false => op,
                }
            })
            .union();
        while let Some((_, v)) = stream.next() {
            let (_, locs) = self.by_word_vec.get(v[0].value as usize).unwrap();
            pre_filtered.extend(locs);
        }
        let res = pre_filtered
            .par_iter()
            .filter_map(|key| {
                let loc = self.all.get(key).unwrap();
                loc.search(st)
                    .map(|score| match score.score > SEARCH_INCLUSION_THRESHOLD {
                        true => Some((*key, score)),
                        false => None,
                    })
            })
            .flatten()
            .collect::<UstrMap<_>>();
        let res_graph = ResultsGraph::from_results(res, &self);
        let mut res = res_graph.scores.into_iter().collect::<Vec<_>>();
        res.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        res.truncate(st.limit);
        res
    }
}

pub fn parse_data_files(data_dir: PathBuf) -> LocationsDb {
    let files = vec![
        "state.json",
        "subdivision.json",
        "locode.json",
        "iata.json",
        "ISO-3166-2:GB.json",
    ];
    let start = Instant::now();
    let db = LocationsDb::default();
    let db = RwLock::new(db);
    files.into_par_iter().for_each(|file| {
        let path = data_dir.join(file);
        info!("Path {path:?}");
        let fo = File::open(path).expect("cannot open json file");
        let reader = BufReader::new(fo);
        let json: serde_json::Value = serde_json::from_reader(reader).expect("cannot decode json");
        info!("Decode json file {file}: {:.2?}", start.elapsed());
        match json {
            Value::Object(obj) => {
                let iter = obj.into_iter().par_bridge();
                let codes = iter
                    .filter_map(|(id, val)| {
                        let raw_any = serde_json::from_value::<AnyLocation>(val)
                            .expect("Cannot decode location code");
                        let loc = Location::from_raw(raw_any);
                        match loc {
                            Ok(loc) => Some(loc),
                            Err(err) => {
                                error!("Error for: {id} {:?}", err);
                                None
                            }
                        }
                    })
                    .for_each(|l| {
                        let mut db = db.write().expect("cannot aquire lock");
                        db.insert(l);
                    });
                info!("{file} decoded to native structs: {:.2?}", start.elapsed());
                codes
            }
            other => panic!("Expected a JSON object: {:?}", other),
        }
    });
    let mut db = db.into_inner().expect("rw lock extract");
    let csv_file = data_dir.join("code-list_csv.csv");
    let csv_file_open = File::open(csv_file).expect("Read CSV File");
    let mut csv_reader = ReaderBuilder::new().from_reader(csv_file_open);
    let iter = csv_reader.deserialize::<CsvLocode>();
    let mut nerrs = 1;
    for rec in iter {
        let csv_loc = rec.expect("CSV Locode decode");
        let key = &csv_loc.key();
        match db.all.get_mut(key) {
            None => {
                debug!("#{} LOCODE not found in db: {} {:?}", nerrs, key, csv_loc);
                nerrs += 1;
            }
            Some(loc) => {
                let coord = csv_loc.parse_coordinates();
                match loc.data {
                    LocData::Locd(mut d) => d.coordinages = coord,
                    _ => panic!("should not happen"),
                }
            }
        }
    }
    let count = db.all.len();
    info!("parsed {} locations in: {:.2?}", count, start.elapsed());
    db.mk_fst()
}
