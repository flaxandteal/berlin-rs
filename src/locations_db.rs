use std::boxed::Box;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Instant;
use std::cmp::min;

use csv::ReaderBuilder;
use fst::{Automaton, Streamer};
use indextree::{Arena, NodeId};
use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelBridge,
    ParallelIterator,
};
use serde_json::Value;
use tracing::{debug, info};
use ustr::{Ustr, UstrMap, UstrSet};

use crate::graph::ResultsGraph;
use crate::location::{AnyLocation, CsvLocode, LocData, Location};
use crate::search::{Score, SearchTerm};
use crate::SEARCH_INCLUSION_THRESHOLD;
use crate::LEV_3_LENGTH_MAX;
use crate::LEV_2_LENGTH_MAX;

#[derive(Default)]
pub struct LocationsDb {
    pub all: UstrMap<Location>,
    pub indices: UstrMap<NodeId>,
    // state names by code
    pub state_by_code: UstrMap<Ustr>,
    // key is in format "gb:lon", value is name
    pub subdiv_by_code: UstrMap<Ustr>,
    pub by_word_map: UstrMap<UstrSet>,
    pub by_word_vec: Vec<(Ustr, UstrSet)>,
    pub fst: fst::Map<Vec<u8>>,
    pub arena: Arena<Ustr>,
}

impl LocationsDb {
    pub fn retrieve(&self, matchable: &str) -> Option<Location> {
        match Ustr::from_existing(matchable) {
            Some(u) => match matchable.len() {
                0 | 1 => None,
                _ => self.all.get(&u).cloned(),
            },
            None => None,
        }
    }
    pub fn insert(&mut self, l: Location) {
        match &l.data {
            LocData::St(s) => {
                self.state_by_code.insert(s.alpha2, l.key);
            }
            LocData::Subdv(_sd) => {
                self.subdiv_by_code.insert(l.id, l.key);
            }
            LocData::Locd(_) => {}
            LocData::Airp(_) => {}
        }
        let node_id = self.arena.new_node(l.key);
        self.indices.insert(l.key, node_id);
        self.all.insert(l.key, l);
    }
    pub fn mk_fst(mut self) -> Self {
        let mut words_map: UstrMap<UstrSet> = UstrMap::default();
        let arena = &mut self.arena;
        self.all.iter().for_each(|(key, loc)| {
            let node_id: &NodeId = self.indices.get(key).unwrap();
            match loc.get_parents() {
                (_, Some(subdiv)) => self.indices.get(&subdiv).unwrap().append(*node_id, arena),
                (Some(st), None) => self.indices.get(&st).unwrap().append(*node_id, arena),
                (None, None) => (),
            };

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
            arena: self.arena,
            indices: self.indices,
            state_by_code: self.state_by_code,
            subdiv_by_code: self.subdiv_by_code,
            by_word_map: words_map,
            by_word_vec: words_vec,
            fst,
        }
    }
    pub fn search<'c>(&'c self, st: &'c SearchTerm) -> Vec<(Ustr, Score)> {
        let fst = &self.fst;
        let search_action = |op: fst::map::OpBuilder<'c>, term: &'c str| match term.len() > 3 {
            true => {
                let prefix_matcher = fst::automaton::Str::new(term).starts_with();
                let lev_dist = match term.chars().count() {
                    count if count < LEV_3_LENGTH_MAX => st.lev_dist,
                    count if count < LEV_2_LENGTH_MAX => min(st.lev_dist, 2),
                    _ => min(st.lev_dist, 1)
                };
                let autom = fst::automaton::Levenshtein::new(term, lev_dist)
                    .expect("build automaton")
                    .union(prefix_matcher);
                op.add(fst.search(autom))
            }
            false => op,
        };

        let grab_action = |term: &Ustr| self.by_word_map.get(term);

        // Grab is for strings we believe we know, searches for those
        // we do not. This allows fast resolution, without searching,
        // where possible.
        let (builder, mut pre_filtered) =
            st.build_search(fst::map::OpBuilder::new(), search_action, grab_action);

        // Finalize and consume the search, extending the prefiltered
        // locations that we wish to apply to.
        let mut stream = builder.union();
        while let Some((_, v)) = stream.next() {
            let (_, locs) = self.by_word_vec.get(v[0].value as usize).unwrap();
            pre_filtered.extend(locs);
        }

        // Search then properly qualifies and quantifies the preliminary
        // matching above.
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

pub fn parse_data_list<I>(mut db: LocationsDb, iter: I) -> Result<LocationsDb, Box<dyn Error>>
where
    I: Iterator,
    I::Item: Into<CsvLocode>,
{
    let mut nerrs = 1;
    for csv_loc in iter {
        let csv_loc: CsvLocode = csv_loc.into();
        let key = &csv_loc.key();
        match db.all.get_mut(key) {
            None => {
                debug!("#{} LOCODE not found in db: {} {:?}", nerrs, key, csv_loc);
                nerrs += 1;
            }
            Some(loc) => {
                let coord = csv_loc.parse_coordinates();
                match loc.data {
                    LocData::Locd(mut d) => d.coordinates = coord,
                    _ => {
                        return Err("should not happen".into());
                    }
                }
            }
        }
    }
    Ok(db)
}

pub fn parse_data_block(
    db: &RwLock<LocationsDb>,
    obj: serde_json::Map<std::string::String, serde_json::Value>,
) -> Result<&RwLock<LocationsDb>, Box<dyn Error>> {
    let iter = obj.into_iter().par_bridge();
    let errors: Vec<String> = iter
        .map(|(id, val)| {
            let raw_any = match serde_json::from_value::<AnyLocation>(val) {
                Ok(val) => val,
                Err(err) => {
                    return Err(format!("\t{id} Cannot decode location code: {:?}", err));
                }
            };
            let loc = Location::from_raw(raw_any);
            match loc {
                Ok(loc) => Ok(loc),
                Err(err) => Err(format!("\t{id} {:?}", err)),
            }
        })
        .filter_map(|l| match l {
            Ok(l) => {
                let mut db = db.write().expect("cannot aquire lock");
                db.insert(l);
                None
            }
            Err(err) => Some(err),
        })
        .collect();
    if errors.len() > 0 {
        Err(format!("Parsing errors:\n{}", errors.join("\n")).into())
    } else {
        Ok(db)
    }
}

pub fn parse_data_files(data_dir: PathBuf) -> Result<LocationsDb, Box<dyn Error>> {
    let files = vec![
        "state.json",
        "subdivision.json",
        "locode.json",
        "iata.json",
        "ISO-3166-2:GB.json",
    ];
    let start = Instant::now();
    let json_blocks = files.into_par_iter().map(|file| {
        let path = data_dir.join(file);
        info!("Path {path:?}");
        let fo = File::open(path).expect("cannot open json file");
        let reader = BufReader::new(fo);
        let json: serde_json::Value = serde_json::from_reader(reader).expect("cannot decode json");
        info!("Decode json file {file}: {:.2?}", start.elapsed());
        (file.to_string(), json)
    });
    let mut db = parse_data_blocks(json_blocks, Some(start))?;
    let csv_file = data_dir.join("code-list_csv.csv");
    let csv_file_open = File::open(csv_file).expect("Read CSV File");
    let mut csv_reader = ReaderBuilder::new().from_reader(csv_file_open);
    let iter = csv_reader.deserialize::<CsvLocode>();
    db = parse_data_list(db, iter.map(|rec| rec.expect("CSV Locode decode")))?;
    let count = db.all.len();
    info!("parsed {} locations in: {:.2?}", count, start.elapsed());
    Ok(db.mk_fst())
}

pub fn parse_data_blocks<'a, I>(
    json_blocks: I,
    start: Option<Instant>,
) -> Result<LocationsDb, Box<dyn Error>>
where
    I: IndexedParallelIterator,
    I::Item: Into<(String, serde_json::Value)>,
{
    let start = match start {
        Some(start) => start,
        None => Instant::now(),
    };
    let db = LocationsDb::default();
    let db = RwLock::new(db);
    let errors = json_blocks
        .into_par_iter()
        .filter_map(|rf| -> Option<String> {
            let (loc, json): (String, serde_json::Value) = rf.into();
            match json {
                Value::Object(obj) => {
                    if let Err(e) = parse_data_block(&db, obj) {
                        return Some(format!("{loc}: {}", e.to_string()));
                    }
                    info!("file decoded to native structs: {:.2?}", start.elapsed());
                    None
                }
                other => Some(format!("{loc}: Expected a JSON object: {:?}", other)),
            }
        })
        .collect::<Vec<String>>();
    if errors.len() > 0 {
        return Err(format!("Blocks failed:\n{}", errors.join("\n")).into());
    }
    Ok(db.into_inner().expect("rw lock extract"))
}
