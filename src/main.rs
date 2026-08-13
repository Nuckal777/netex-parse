use std::{io::Write, path::PathBuf};

use clap::{Parser, ValueEnum};
use indicatif::ParallelProgressIterator;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use zip::ZipArchive;

use crate::{
    graph::{ValidDays, WalkEdge},
    parser::NetexData,
};

mod graph;
mod parser;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum OutputFormat {
    Csv,
    Binary,
}

/// Multi-threaded parser for public transport information in the netex format
/// that outputs graphs for stations and timetables
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Path to a zip file containing netex documents.
    #[clap()]
    netex_file: PathBuf,

    /// Output format. CSV creates a nodes.csv and edges.csv file.
    #[clap(short, long, value_parser, value_enum)]
    output_format: OutputFormat,

    /// Path to json file containing walkway durations between stations.
    #[clap(short, long)]
    walkways: Option<PathBuf>,

    /// Substring the netex documents file names must included.
    #[clap(short, long, default_value = "")]
    filter: String,

    /// Date (YYYY-MM-DD) from which the dataset can be considered valid.
    #[clap(long, default_value = "2000-01-01")]
    from: String,

    /// Date (YYYY-MM-DD) to which the dataset can be considered valid.
    #[clap(long, default_value = "2100-01-01")]
    to: String,
}

fn main() {
    let args = Args::parse();
    let zip_stream = std::fs::File::open(&args.netex_file).expect("failed to open data");
    let zip_memmap = unsafe { memmap2::Mmap::map(&zip_stream).expect("failed mmap") };
    let zip_cursor = std::io::Cursor::new(&zip_memmap);
    let archive = ZipArchive::new(zip_cursor).expect("failed to read zip");
    let documents: Vec<String> = archive
        .file_names()
        .filter(|f| f.contains(&args.filter))
        .map(str::to_owned)
        .collect();
    let walkways: Vec<WalkEdge> = match args.walkways {
        None => vec![],
        Some(path) => {
            println!("loading walk data");
            let walk_bytes = std::fs::read(path).expect("failed to read walk data");
            serde_json::from_slice(&walk_bytes).expect("failed to deserialize json")
        }
    };
    let graph = parse(&zip_memmap, &documents, &walkways);
    println!(
        "{} has {} deduped nodes and {} deduped edges.",
        args.filter,
        graph.nodes.len(),
        graph.edges.len(),
    );
    let metadata = graph::Metadata {
        name: args.netex_file.to_string_lossy().to_string(),
        from: parser::NetexData::parse_date(&args.from),
        to: parser::NetexData::parse_date(&args.to),
    };
    match args.output_format {
        OutputFormat::Csv => dump_csv(&graph).expect("failed to dump csv"),
        OutputFormat::Binary => dump_binary(&metadata, &graph).expect("failed to dump binary"),
    }
}

fn parse(archive: &memmap2::Mmap, documents: &[String], walkways: &[WalkEdge]) -> graph::Graph {
    let mut data = documents
        .par_iter()
        .progress_count(documents.len() as u64)
        .map(|doc| {
            let zip_cursor = std::io::Cursor::new(archive);
            let mut archive = ZipArchive::new(zip_cursor).expect("failed to read zip");
            let file = match archive.by_name(doc) {
                Ok(f) => f,
                Err(err) => {
                    println!("{doc} had err: {err}");
                    return Vec::new();
                }
            };
            if file.is_dir() {
                return Vec::new();
            }
            let size = file.size().try_into().expect("u64 does not fit usize");
            vec![parser::NetexData::from_xml(file, size).unwrap_or_default()]
        })
        .reduce(Vec::<NetexData>::new, |mut accum, item| {
            accum.extend(item);
            accum
        });
    println!("deduping...");
    for d in &mut data {
        d.scheduled_stop_points.retain(|stop| {
            stop.long > 5.5 && stop.long < 15.5 && stop.lat > 47.0 && stop.lat < 55.5
        });
    }
    graph::Graph::from_data(&data, walkways)
}

fn dump_csv(graph: &graph::Graph) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    let mut node_writer = std::io::BufWriter::new(opts.open("./nodes.csv")?);
    for node in &graph.nodes {
        node_writer.write_all(
            format!(
                "\"{}\",{},{},{}\n",
                node.short_name.replace('"', "'"),
                node.long,
                node.lat,
                node.id
            )
            .as_bytes(),
        )?;
    }
    node_writer.flush()?;

    let mut edge_writer = std::io::BufWriter::new(opts.open("./edges.csv")?);
    for edge in &graph.edges {
        let timetable = serde_json::to_string(&edge.timetable)
            .expect("failed to serialize json")
            .replace('"', "\\\"");
        edge_writer.write_all(
            format!(
                "\"{}\",\"{}\",\"{}\"\n",
                graph.nodes[edge.start_node].short_name,
                graph.nodes[edge.end_node].short_name,
                timetable
            )
            .as_bytes(),
        )?;
    }
    edge_writer.flush()?;
    Ok(())
}

#[derive(serde::Serialize)]
struct MetaNode {
    name: String,
    // large u64 do not survive JSON.parse over in
    // the JS world, so use a string here
    id: String,
    coords: [f32; 2],
}

#[allow(clippy::cast_possible_truncation)]
fn dump_binary(
    metdata: &graph::Metadata,
    graph: &graph::Graph,
) -> Result<(), Box<dyn std::error::Error>> {
    fn metdata_as_bytes(metdata: &graph::Metadata) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        writer.write_all(&metdata.from.to_le_bytes())?;
        writer.write_all(&metdata.to.to_le_bytes())?;
        let name_len: u16 = metdata.name.len().try_into()?;
        writer.write_all(&name_len.to_le_bytes())?;
        writer.write_all(metdata.name.as_bytes())?;
        Ok(data)
    }

    fn node_as_bytes(node: &graph::Node) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // index is implicit
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        writer.write_all(&node.id.to_le_bytes())?;
        writer.write_all(&node.lat.to_le_bytes())?;
        writer.write_all(&node.long.to_le_bytes())?;
        let name_bytes = node.short_name.as_bytes();
        let name_len: u16 = name_bytes.len().try_into()?;
        writer.write_all(&name_len.to_le_bytes())?;
        writer.write_all(name_bytes)?;
        Ok(data)
    }

    fn valid_days_to_bytes(
        valid_days: &[ValidDays],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        let valid_day_len: u16 = valid_days
            .len()
            .try_into()
            .expect("valid_day_len too large :(");
        writer.write_all(&valid_day_len.to_le_bytes())?;
        for vd in valid_days {
            let vd_len: u8 = vd.bits.len().try_into()?;
            writer.write_all(&[vd_len])?;
            writer.write_all(&vd.bits)?;
        }
        Ok(data)
    }

    fn period_as_bytes(
        period: &graph::OperatingPeriod,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        writer.write_all(&period.from.to_le_bytes())?;
        writer.write_all(&period.to.to_le_bytes())?;
        let valid_day_idx: u16 = period.valid_days_idx.try_into()?;
        writer.write_all(&valid_day_idx.to_le_bytes())?;
        Ok(data)
    }

    fn journeys_as_bytes(
        journeys: &[graph::Journey],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        for journey in journeys {
            writer.write_all(&encode_arrival_departure(
                journey.arrival,
                journey.departure,
            ))?;
            write_period_index(&mut writer, journey.operating_period)?;
        }
        Ok(data)
    }

    fn write_period_index<T: std::io::Write>(
        writer: &mut T,
        operating_period: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if operating_period < 128 {
            writer.write_all(&[operating_period as u8])?;
            return Ok(());
        }
        let out1 = 0x80 | ((operating_period >> 8) as u8 & 0x7F);
        let out2 = operating_period as u8;
        writer.write_all(&[out1, out2])?;
        Ok(())
    }

    fn edge_as_bytes(edge: &graph::Edge) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut data = Vec::<u8>::new();
        let mut writer = std::io::Cursor::new(&mut data);
        writer.write_all(&(edge.start_node as u32).to_le_bytes())?;
        writer.write_all(&(edge.end_node as u32).to_le_bytes())?;
        writer.write_all(&edge.walk_seconds.to_le_bytes())?;
        let journeys_data = journeys_as_bytes(&edge.timetable.journeys)?;
        let journeys_len: u32 = journeys_data.len().try_into()?;
        writer.write_all(&journeys_len.to_le_bytes())?;
        writer.write_all(&journeys_data)?;
        let mut periods = Vec::<u8>::new();
        for period in &edge.timetable.periods {
            periods.extend(period_as_bytes(period)?);
        }
        let periods_len: u16 = periods.len().try_into()?;
        writer.write_all(&periods_len.to_le_bytes())?;
        writer.write_all(&periods)?;
        Ok(data)
    }

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    let mut writer = std::io::BufWriter::new(opts.open("./graph.bin")?);
    // TODO: magic number, file version
    // nodes with data
    let metadata_data = metdata_as_bytes(metdata)?;
    let metadata_len: u16 = metadata_data.len().try_into()?;
    writer.write_all(&metadata_len.to_le_bytes())?;
    writer.write_all(&metadata_data)?;

    let mut node_data = Vec::<u8>::new();
    let mut node_writer = std::io::Cursor::new(&mut node_data);
    for node in &graph.nodes {
        node_writer.write_all(&node_as_bytes(node)?)?;
    }
    writer.write_all(&(graph.nodes.len() as u32).to_le_bytes())?;
    writer.write_all(&node_data)?;

    writer.write_all(&valid_days_to_bytes(&graph.valid_days)?)?;
    // edges with data
    let mut edge_data = Vec::<u8>::new();
    let mut edge_writer = std::io::Cursor::new(&mut edge_data);
    for edge in &graph.edges {
        edge_writer.write_all(&edge_as_bytes(edge)?)?;
    }
    writer.write_all(&(graph.edges.len() as u32).to_le_bytes())?;
    writer.write_all(&edge_data)?;
    writer.flush()?;

    let metas: Vec<MetaNode> = graph
        .nodes
        .iter()
        .map(|n| MetaNode {
            coords: [n.long, n.lat],
            id: n.id.to_string(),
            name: n.short_name.clone(),
        })
        .collect();
    std::fs::write("nodes.json", serde_json::to_vec(&metas)?)?;
    Ok(())
}

fn encode_arrival_departure(arrival: u16, departure: u16) -> [u8; 3] {
    let mut result = [0_u8, 0, 0];
    result[0] = arrival.to_le_bytes()[0];
    result[1] = arrival.to_le_bytes()[1] << 4;
    result[2] = departure.to_le_bytes()[0];
    result[1] |= departure.to_le_bytes()[1] & 0xF;
    result
}

#[cfg(test)]
mod tests {
    use crate::encode_arrival_departure;

    fn decode_arrival_departure(data: [u8; 3]) -> (u16, u16) {
        let arrival = u16::from(data[0]) | u16::from(data[1] & 0xF0) << 4;
        let departure = u16::from(data[2]) | u16::from(data[1] & 0xF) << 8;
        (arrival, departure)
    }

    #[test]
    fn arrival_departure_roundtrip1() {
        let data = encode_arrival_departure(1025, 1430);
        let (a, b) = decode_arrival_departure(data);
        assert_eq!(a, 1025);
        assert_eq!(b, 1430);
    }

    #[test]
    fn arrival_departure_roundtrip2() {
        let data = encode_arrival_departure(240, 579);
        let (a, b) = decode_arrival_departure(data);
        assert_eq!(a, 240);
        assert_eq!(b, 579);
    }
}
