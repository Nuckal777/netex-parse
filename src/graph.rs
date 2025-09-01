use std::collections::HashMap;

use base64::engine::Engine;
use geo::{Centroid, Destination, Haversine};
use indicatif::ParallelProgressIterator;
use rayon::iter::{IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator};

use crate::parser::{
    Authority, DayTypeAssignment, Line, NetexData, ServiceJourney, UicOperatingPeriod,
};

#[derive(Clone, Default, Debug)]
pub struct Node {
    pub id: u64,
    pub short_name: String,
    pub long: f32,
    pub lat: f32,
}

#[derive(Debug, serde::Serialize)]
pub struct Journey {
    #[serde(rename(serialize = "d"))]
    pub departure: u16,
    #[serde(rename(serialize = "a"))]
    pub arrival: u16,
    #[serde(rename(serialize = "t"))]
    pub transport_mode: String,
    #[serde(rename(serialize = "o"))]
    pub operating_period: usize,
    #[serde(rename(serialize = "l"))]
    pub line: String,
    #[serde(rename(serialize = "c"))]
    pub controller: String,
}

#[derive(Clone, Default, Debug, serde::Serialize)]
pub struct OperatingPeriod {
    #[serde(rename(serialize = "f"))]
    pub from: u16,
    #[serde(rename(serialize = "t"))]
    pub to: u16,
    #[serde(rename(serialize = "v"))]
    pub valid_day_bits: String,
    pub valid_day: Vec<u8>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Timetable {
    #[serde(rename(serialize = "j"))]
    pub journeys: Vec<Journey>,
    #[serde(rename(serialize = "p"))]
    pub periods: Vec<OperatingPeriod>,
}

#[derive(Debug)]
pub struct Edge {
    pub start_node: usize,
    pub end_node: usize,
    pub timetable: Timetable,
    pub walk_seconds: u16,
}

#[derive(Debug)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Indices {
    node: usize,
    data: usize,
    stop: usize,
}

#[derive(Default, serde::Deserialize)]
pub struct WalkEdge {
    pub start: u64,
    pub end: u64,
    pub duration: f32,
}

struct Nodes {
    vec: Vec<Node>,
    // netex ref to indices
    ref_map: HashMap<u64, Indices>,
    // calculate id to indices
    id_map: HashMap<u64, usize>,
    // name_map: HashMap<String, Indices>,
}

impl Nodes {
    fn from_data(data: &[NetexData]) -> Nodes {
        let mut nodes_by_name = HashMap::<&str, Vec<Indices>>::new();
        let mut ref_to_node_idx = HashMap::<u64, Indices>::new();
        for (data_idx, one_data) in data.iter().enumerate() {
            for (stop_idx, stop) in one_data.scheduled_stop_points.iter().enumerate() {
                let indices = Indices {
                    data: data_idx,
                    node: 0,
                    stop: stop_idx,
                };
                nodes_by_name
                    .entry(&stop.short_name)
                    .and_modify(|stops| stops.push(indices))
                    .or_insert(vec![indices]);
            }
        }
        let mut id_map = HashMap::<u64, usize>::new();
        let mut nodes: Vec<Node> = vec![];
        let distance = 1000.0; // radius in meters
        for (name, stops) in nodes_by_name {
            type TreeObj<'a> = rstar::primitives::GeomWithData<geo::Coord<f32>, Indices>;
            let mut tree = rstar::RTree::<TreeObj>::bulk_load(
                stops
                    .iter()
                    .map(|indices| {
                        let stop = &data[indices.data].scheduled_stop_points[indices.stop];
                        TreeObj::new(geo::Coord::from([stop.long, stop.lat]), *indices)
                    })
                    .collect(),
            );
            while tree.size() > 0 {
                let indices = tree.iter().next().unwrap().data;
                let current = &data[indices.data].scheduled_stop_points[indices.stop];
                let center = geo::Point::from((current.long, current.lat));
                let corner1 = Haversine.destination(center, 45.0, distance);
                let corner2 = Haversine.destination(center, 225.0, distance);
                let aabb =
                    rstar::AABB::<geo::Coord<f32>>::from_corners(corner1.into(), corner2.into());
                let local: Vec<TreeObj> = tree.drain_in_envelope(aabb).collect();
                for point in &local {
                    let point_idx = point.data;
                    let point_stop = &data[point_idx.data].scheduled_stop_points[point_idx.stop];
                    ref_to_node_idx.insert(
                        point_stop.id,
                        Indices {
                            node: nodes.len(),
                            data: point_idx.data,
                            stop: point_idx.stop,
                        },
                    );
                }
                let coords: Vec<geo::Coord<f32>> =
                    local.iter().map(TreeObj::geom).copied().collect();
                let centroid = geo::LineString::new(coords)
                    .centroid()
                    .expect("failed to calculate centroid");
                // current.id is not consistent across runs
                // there are different scheduled_point_stops with the same name + coords that are different entities
                // so xor all stops in a cluster, risking hash collisions
                let mut node_ids: Vec<u64> = local
                    .iter()
                    .map(|val| data[val.data.data].scheduled_stop_points[val.data.stop].id)
                    .collect();
                node_ids.sort_unstable();
                node_ids.dedup();
                let node_id = node_ids.into_iter().reduce(|l, r| l ^ r).unwrap();
                id_map.insert(node_id, nodes.len());
                nodes.push(Node {
                    lat: centroid.y(),
                    long: centroid.x(),
                    short_name: name.to_owned(),
                    id: node_id,
                });
            }
        }
        Nodes {
            id_map,
            vec: nodes,
            ref_map: ref_to_node_idx,
        }
    }

    fn get(&self, idx: usize) -> &Node {
        &self.vec[idx]
    }

    fn index_by_id(&self, id: u64) -> Option<usize> {
        self.id_map.get(&id).copied()
    }

    fn index_by_stop_ref(&self, stop_ref: u64) -> Option<&Indices> {
        self.ref_map.get(&stop_ref)
    }
}

pub struct JourneyTransformer {
    authorities: HashMap<u64, Authority>,
    day_type_assignments: HashMap<u64, DayTypeAssignment>,
    lines: HashMap<u64, Line>,
    pattern_ref_to_line_ref: HashMap<u64, u64>,
    point_in_journey_to_stop_ref: HashMap<u64, u64>,
    period_map: HashMap<u64, usize>,
}

impl JourneyTransformer {
    fn from_data(data: &[NetexData]) -> JourneyTransformer {
        let mut point_in_journey_to_stop_ref = HashMap::<u64, u64>::new();
        for one_data in data {
            for sequence in &one_data.service_journey_patterns {
                for stop in &sequence.stops {
                    point_in_journey_to_stop_ref
                        .entry(stop.id)
                        .or_insert(stop.scheduled_stop_point);
                }
            }
        }

        let mut lines = HashMap::<u64, Line>::new();
        for one_data in data {
            for line in &one_data.lines {
                lines.insert(line.id, line.clone());
            }
        }

        let mut authorities = HashMap::<u64, Authority>::new();
        for one_data in data {
            for authority in &one_data.authorities {
                authorities.insert(authority.id, authority.clone());
            }
        }

        let mut pattern_ref_to_line_ref = HashMap::<u64, u64>::new();
        for one_data in data {
            for journey_pattern in &one_data.service_journey_patterns {
                pattern_ref_to_line_ref.insert(journey_pattern.id, journey_pattern.line);
            }
        }

        let mut period_map = HashMap::<u64, usize>::new();
        for (idx, period) in data
            .iter()
            .flat_map(|d| d.operating_periods.iter())
            .enumerate()
        {
            period_map.insert(period.id, idx);
        }
        let mut day_type_assignments = HashMap::<u64, DayTypeAssignment>::new();
        for dta in data.iter().flat_map(|d| d.day_type_assignments.iter()) {
            day_type_assignments.insert(dta.day_type, dta.clone());
        }

        JourneyTransformer {
            authorities,
            day_type_assignments,
            lines,
            pattern_ref_to_line_ref,
            point_in_journey_to_stop_ref,
            period_map,
        }
    }

    fn to_edges(&self, journey: &ServiceJourney, nodes: &Nodes) -> HashMap<(usize, usize), Edge> {
        let mut local_edges = HashMap::<(usize, usize), Edge>::new();
        for window in journey.passing_times.windows(2) {
            let pre = &window[0];
            let current = &window[1];
            let Some(start_indecies) = nodes.index_by_stop_ref(
                self.point_in_journey_to_stop_ref[&pre.stop_point_in_journey_pattern],
            ) else {
                continue;
            };
            let Some(end_indecies) = nodes.index_by_stop_ref(
                self.point_in_journey_to_stop_ref[&current.stop_point_in_journey_pattern],
            ) else {
                continue;
            };
            let period = self
                .day_type_assignments
                .get(&journey.day_type)
                .expect("Day type without operating period found")
                .operating_period;

            let entry = local_edges
                .entry((start_indecies.node, end_indecies.node))
                .or_insert(Edge {
                    walk_seconds: u16::MAX,
                    start_node: start_indecies.node,
                    end_node: end_indecies.node,
                    timetable: Timetable::default(),
                });
            let line = &self.lines[&self.pattern_ref_to_line_ref[&journey.pattern_ref]];
            entry.timetable.journeys.push(Journey {
                departure: pre.departure,
                arrival: current.arrival,
                transport_mode: journey.transport_mode.clone(),
                operating_period: *self.period_map.get(&period).unwrap(),
                line: line.short_name.clone(),
                controller: self.authorities[&line.authority].short_name.clone(),
            });
        }
        local_edges
    }
}

impl Graph {
    pub fn from_data(data: &[NetexData], walk_seconds: &[WalkEdge]) -> Graph {
        // nodes contains stops deduplicated by short name
        let nodes = Nodes::from_data(data);
        let journey_transformer = JourneyTransformer::from_data(data);

        let mut edges = data
            .par_iter()
            .progress()
            .flat_map(|d| d.service_journeys.par_iter())
            .map(|journey| journey_transformer.to_edges(journey, &nodes))
            .reduce(HashMap::<(usize, usize), Edge>::new, |a, mut b| {
                for (key, value) in a {
                    let entry = b.entry(key).or_insert(Edge {
                        walk_seconds: u16::MAX,
                        start_node: key.0,
                        end_node: key.1,
                        timetable: Timetable::default(),
                    });
                    entry
                        .timetable
                        .journeys
                        .extend(value.timetable.journeys.into_iter());
                }
                b
            });

        // loop through walk seconds hashmap
        for walk_edge in walk_seconds {
            Self::update_walk(walk_edge, &nodes, &mut edges);
        }

        // filter non-sensical journeys
        edges.par_iter_mut().for_each(|(_, edge)| {
            Self::filter_journeys(edge, &nodes);
        });

        // map global operating period index to local index
        edges.par_iter_mut().for_each(|(_, edge)| {
            let mut global_to_local = HashMap::<usize, usize>::new();
            let mut counter = 0;
            for journey in &edge.timetable.journeys {
                if global_to_local.contains_key(&journey.operating_period) {
                    continue;
                }
                global_to_local.insert(journey.operating_period, counter);
                counter += 1;
            }
            let mut local_ops = vec![OperatingPeriod::default(); global_to_local.len()];
            for (global, local) in &global_to_local {
                let uic_op = Self::lookup_operating_period(data, *global).expect(
                    "failed to map global operating period index to concrete operating period",
                );
                local_ops[*local] = OperatingPeriod {
                    from: uic_op.from,
                    to: uic_op.to,
                    valid_day_bits: base64::engine::general_purpose::STANDARD
                        .encode(&uic_op.valid_day_bits),
                    valid_day: uic_op.valid_day_bits.clone(),
                }
            }
            for journey in &mut edge.timetable.journeys {
                journey.operating_period = *global_to_local
                    .get(&journey.operating_period)
                    .expect("failed to map global to local operating period");
            }
            edge.timetable.periods = local_ops;
        });

        Graph {
            nodes: nodes.vec,
            edges: edges.into_values().collect(),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn update_walk(walk_edge: &WalkEdge, nodes: &Nodes, edges: &mut HashMap<(usize, usize), Edge>) {
        let (Some(start_idx), Some(end_idx)) = (
            nodes.index_by_id(walk_edge.start),
            nodes.index_by_id(walk_edge.end),
        ) else {
            println!("{} -> {} borked", walk_edge.start, walk_edge.end);
            return;
        };
        let start_node = nodes.get(start_idx);
        let end_node = nodes.get(end_idx);
        let distance = great_circle_distance(
            (start_node.long, start_node.lat),
            (end_node.long, end_node.lat),
        );
        if distance > 1.0 {
            return;
        }
        let forward = edges.entry((start_idx, end_idx)).or_insert(Edge {
            start_node: start_idx,
            end_node: end_idx,
            timetable: Timetable::default(),
            walk_seconds: u16::MAX,
        });
        forward.walk_seconds = walk_edge.duration as u16;
        let backward = edges.entry((end_idx, start_idx)).or_insert(Edge {
            start_node: end_idx,
            end_node: start_idx,
            timetable: Timetable::default(),
            walk_seconds: u16::MAX,
        });
        backward.walk_seconds = walk_edge.duration as u16;
    }

    #[allow(clippy::cast_lossless)]
    fn filter_journeys(edge: &mut Edge, nodes: &Nodes) {
        let start_node = nodes.get(edge.start_node);
        let end_node = nodes.get(edge.end_node);
        let distance = great_circle_distance(
            (start_node.long, start_node.lat),
            (end_node.long, end_node.lat),
        );
        edge.timetable.journeys.retain(|j| {
            let departure_min = (j.departure % 60) + ((j.departure / 60) * 60);
            let mut arrival_min = (j.arrival % 60) + ((j.arrival / 60) * 60);
            if arrival_min < departure_min {
                arrival_min += 24 * 60;
            }
            let minutes = arrival_min - departure_min;
            let hours = minutes as f32 / 60.0;
            let speed = distance / hours;
            speed < 325.0 || (minutes < 3 && distance < 3.0)
        });
    }

    fn lookup_operating_period(
        data: &[NetexData],
        mut global_index: usize,
    ) -> Option<&UicOperatingPeriod> {
        for one_data in data {
            if global_index < one_data.operating_periods.len() {
                return Some(&one_data.operating_periods[global_index]);
            }
            global_index -= one_data.operating_periods.len();
        }
        None
    }
}

fn great_circle_distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    use std::f32::consts;
    let a_lon = a.0 * consts::PI / 180.0;
    let a_lat = a.1 * consts::PI / 180.0;
    let b_lon = b.0 * consts::PI / 180.0;
    let b_lat = b.1 * consts::PI / 180.0;

    let diff_lon = (a_lon - b_lon).abs();
    let intermediate = a_lat.sin() * b_lat.sin() + a_lat.cos() * b_lat.cos() * diff_lon.cos();
    let angle = intermediate.acos();
    6371.009 * angle
}
