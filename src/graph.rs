use std::cmp::{max, min};

use petgraph::graphmap::DiGraphMap;
use ustr::{Ustr, UstrMap};

use crate::locations_db::LocationsDb;
use crate::search::Score;
use crate::GRAPH_EDGE_THRESHOLD;

pub struct ResultsGraph {
    pub(crate) scores: UstrMap<Score>,
}

impl ResultsGraph {
    pub fn from_results(mut results: UstrMap<Score>, db: &LocationsDb) -> Self {
        // let start = Instant::now();
        let mut graph: DiGraphMap<Ustr, _> = DiGraphMap::new();
        results.iter().for_each(|(key, score)| {
            let loc = db.all.get(key).expect("location in db");
            graph.add_node(loc.key);
            let (state_key, subdiv_key) = loc.get_parents();
            for key in [state_key, subdiv_key] {
                if let Some(superkey) = key {
                    if let Some(superkey_score) = results.get(&superkey) {
                        if min(superkey_score.score, score.score) > GRAPH_EDGE_THRESHOLD {
                            let weight = (superkey_score.score, score.score);
                            graph.add_edge(superkey, loc.key, weight);
                        }
                    }
                }
            }
        });
        let mut edges = graph.all_edges().collect::<Vec<_>>();
        edges.sort_unstable_by(|a, b| b.2.cmp(a.2));
        edges.into_iter().enumerate().for_each(|(_i, edge)| {
            let loc = db.all.get(&edge.1).unwrap();
            let parent = db.all.get(&edge.0).unwrap();
            let parent_boost = parent.parent_boost(edge.2 .0);
            let total_score = parent_boost + edge.2 .1;
            let old = results.get(&loc.key).cloned().unwrap();
            results.insert(
                loc.key,
                Score {
                    offset: old.offset,
                    score: max(total_score, old.score),
                },
            );
        });
        // info!("Graph analysis in {:.3?}", start.elapsed());
        ResultsGraph { scores: results }
    }
}

#[cfg(test)]
mod tests {
}
