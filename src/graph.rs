use std::collections::HashMap;

use indicatif::ParallelProgressIterator;
use rayon::iter::{IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator};

use crate::parser::{Authority, DayTypeAssignment, Line, NetexData, UicOperatingPeriod};

#[derive(Clone, Default, Debug)]
pub struct Node {
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
    pub from: u32,
    #[serde(rename(serialize = "t"))]
    pub to: u32,
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
}

#[derive(Debug)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Copy)]
struct Indices {
    node: usize,
    data: usize,
    stop: usize,
}

impl Graph {
    pub fn from_data(data: &[NetexData]) -> Graph {
        // short name to scheduled point stop index
        let mut node_map = std::collections::HashMap::<String, Indices>::new();
        let mut ref_to_node_idx = std::collections::HashMap::<u64, Indices>::new();
        let mut counter = 0_usize;
        for (data_idx, one_data) in data.iter().enumerate() {
            for (stop_idx, stop) in one_data.scheduled_stop_points.iter().enumerate() {
                if node_map.contains_key(&stop.short_name) {
                    ref_to_node_idx.insert(stop.id, node_map[&stop.short_name]);
                } else {
                    let indices = Indices {
                        data: data_idx,
                        node: counter,
                        stop: stop_idx,
                    };
                    node_map.insert(stop.short_name.clone(), indices);
                    ref_to_node_idx.insert(stop.id, indices);
                    counter += 1;
                }
            }
        }
        let mut nodes = vec![Node::default(); node_map.len()];
        for idx in node_map.values() {
            let current = &data[idx.data].scheduled_stop_points[idx.stop];
            nodes[idx.node] = Node {
                short_name: current.short_name.clone(),
                long: current.long,
                lat: current.lat,
            };
        }
        // nodes contains stops deduplicated by short name
        // ref_to_node_idx maps a nextex stop ref to a index into nodes

        let mut point_in_journey_to_stop_ref = std::collections::HashMap::<u64, u64>::new();
        for one_data in data {
            for sequence in &one_data.service_journey_patterns {
                for stop in &sequence.stops {
                    point_in_journey_to_stop_ref
                        .entry(stop.id)
                        .or_insert(stop.scheduled_stop_point);
                }
            }
        }

        let mut lines = std::collections::HashMap::<u64, Line>::new();
        for one_data in data {
            for line in &one_data.lines {
                lines.insert(line.id, line.clone());
            }
        }

        let mut authorities = std::collections::HashMap::<u64, Authority>::new();
        for one_data in data {
            for authority in &one_data.authorities {
                authorities.insert(authority.id, authority.clone());
            }
        }

        let mut pattern_ref_to_line = std::collections::HashMap::<u64, u64>::new();
        for one_data in data {
            for journey_pattern in &one_data.service_journey_patterns {
                pattern_ref_to_line.insert(journey_pattern.id, journey_pattern.line);
            }
        }
        
        let mut period_map = std::collections::HashMap::<u64, usize>::new();
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

        let mut edges = data
            .par_iter()
            .progress()
            .flat_map(|d| d.service_journeys.par_iter())
            .map(|journey| {
                let mut local_edges = std::collections::HashMap::<(usize, usize), Edge>::new();
                for window in journey.passing_times.windows(2) {
                    let pre = &window[0];
                    let current = &window[1];
                    let start_node = ref_to_node_idx
                        [&point_in_journey_to_stop_ref[&pre.stop_point_in_journey_pattern]]
                        .node;
                    let end_node = ref_to_node_idx
                        [&point_in_journey_to_stop_ref[&current.stop_point_in_journey_pattern]]
                        .node;
                    let period = day_type_assignments
                        .get(&journey.day_type)
                        .expect("Day type without operating period found")
                        .operating_period;

                    let entry = local_edges.entry((start_node, end_node)).or_insert(Edge {
                        start_node: start_node,
                        end_node: end_node,
                        timetable: Timetable::default(),
                    });
                    let line = &lines[&pattern_ref_to_line[&journey.pattern_ref]];
                    entry.timetable.journeys.push(Journey {
                        departure: pre.departure,
                        arrival: current.arrival,
                        transport_mode: journey.transport_mode.clone(),
                        operating_period: *period_map.get(&period).unwrap(),
                        line: line.short_name.clone(),
                        controller: authorities[&line.authority].short_name.clone(),
                    });
                }
                local_edges
            })
            .reduce(
                std::collections::HashMap::<(usize, usize), Edge>::new,
                |a, mut b| {
                    for (key, value) in a.into_iter() {
                        let entry = b.entry(key).or_insert(Edge {
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
                },
            );

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
                    valid_day_bits: base64::encode(&uic_op.valid_day_bits),
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
            nodes,
            edges: edges.into_iter().map(|(_, e)| e).collect(),
        }
    }

    fn lookup_operating_period(
        data: &[NetexData],
        mut global_index: usize,
    ) -> Option<&UicOperatingPeriod> {
        for one_data in data {
            if global_index < one_data.operating_periods.len() {
                return Some(&one_data.operating_periods[global_index]);
            }
            global_index -= one_data.operating_periods.len()
        }
        None
    }
}
