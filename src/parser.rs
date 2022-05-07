#[derive(Default)]
pub struct DayTypeAssignment {
    pub operating_period: String,
    pub day_type: String,
    pub is_available: bool,
}

#[derive(Clone, Default)]
pub struct UicOperatingPeriod {
    pub id: String,
    pub from: u32,
    pub to: u32,
    pub valid_day_bits: Vec<u8>,
}

#[derive(Default)]
pub struct ScheduledStopPoint {
    pub id: String,
    pub short_name: String,
    pub long: f32,
    pub lat: f32,
}

#[derive(Default)]
pub struct StopPointInJourneyPattern {
    pub id: String,
    pub scheduled_stop_point: String,
}

#[derive(Default)]
pub struct PointsInSequence {
    /// refs for scheduled point stops
    pub stops: Vec<StopPointInJourneyPattern>,
}

#[derive(Default)]
pub struct TimetabledPassingTime {
    pub stop_point_in_journey_pattern: String,
    pub arrival: u16,
    pub departure: u16,
}

#[derive(Default)]
pub struct ServiceJourney {
    pub passing_times: Vec<TimetabledPassingTime>,
    pub day_type: String,
    pub transport_mode: String,
}

#[derive(Default)]
pub struct NetexData {
    pub scheduled_stop_points: Vec<ScheduledStopPoint>,
    pub points_in_squence: Vec<PointsInSequence>,
    pub service_journeys: Vec<ServiceJourney>,
    pub operating_periods: Vec<UicOperatingPeriod>,
    pub day_type_assignments: Vec<DayTypeAssignment>,
}

impl NetexData {
    pub fn from_xml(
        mut read: impl std::io::Read,
        size: usize,
    ) -> Result<NetexData, Box<dyn std::error::Error>> {
        let mut data = NetexData::default();
        let mut buf = Vec::<u8>::with_capacity(size);
        read.read_to_end(&mut buf)?;
        let text = unsafe { String::from_utf8_unchecked(buf) };
        let document = roxmltree::Document::parse(&text)?;

        let nodes: Result<Vec<ScheduledStopPoint>, Box<dyn std::error::Error>> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "ScheduledStopPoint")
            .map(|node| NetexData::parse_scheduled_stop_point(&node))
            .collect();
        data.scheduled_stop_points = nodes?;

        let points: Vec<PointsInSequence> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "pointsInSequence")
            .map(|node| NetexData::parse_points_in_sequence(&node))
            .collect();
        data.points_in_squence = points;

        let journeys: Vec<ServiceJourney> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "ServiceJourney")
            .map(|node| NetexData::parse_service_journey(&node))
            .collect();
        data.service_journeys = journeys;

        let operating_periods: Vec<UicOperatingPeriod> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "UicOperatingPeriod")
            .map(|node| NetexData::parse_operating_period(&node))
            .collect();
        data.operating_periods = operating_periods;

        let day_type_assignments: Result<Vec<DayTypeAssignment>, Box<dyn std::error::Error>> =
            document
                .descendants()
                .filter(|node| node.tag_name().name() == "DayTypeAssignment")
                .map(|node| NetexData::parse_day_type_assignment(&node))
                .collect();
        data.day_type_assignments = day_type_assignments?;
        Ok(data)
    }

    fn parse_scheduled_stop_point(
        node: &roxmltree::Node,
    ) -> Result<ScheduledStopPoint, Box<dyn std::error::Error>> {
        let mut result = ScheduledStopPoint {
            id: node.attribute("id").unwrap_or_default().to_owned(),
            ..ScheduledStopPoint::default()
        };
        for child in node.descendants() {
            match child.tag_name().name() {
                "ShortName" => result.short_name = child.text().unwrap_or_default().replace('"', ""),
                "Longitude" => result.long = child.text().unwrap_or_default().parse::<f32>()?.clamp(-180.0, 180.0),
                "Latitude" => result.lat = child.text().unwrap_or_default().parse::<f32>()?.clamp(-90.0, 90.0),
                _ => {}
            }
        }
        Ok(result)
    }

    fn parse_points_in_sequence(node: &roxmltree::Node) -> PointsInSequence {
        let mut result = PointsInSequence::default();
        for sub_node in node.descendants() {
            if sub_node.tag_name().name() != "StopPointInJourneyPattern" {
                continue;
            }
            let mut stop = StopPointInJourneyPattern {
                id: sub_node.attribute("id").unwrap_or_default().to_owned(),
                ..StopPointInJourneyPattern::default()
            };
            stop.scheduled_stop_point = sub_node
                .descendants()
                .find(|child| child.tag_name().name() == "ScheduledStopPointRef")
                .map(|node| node.attribute("ref").unwrap_or_default().to_owned())
                .unwrap_or_default();
            result.stops.push(stop);
        }
        result
    }

    fn parse_service_journey(node: &roxmltree::Node) -> ServiceJourney {
        let day_type = node
            .descendants()
            .find(|node| node.tag_name().name() == "DayTypeRef")
            .unwrap()
            .attribute("ref")
            .unwrap_or_default();
        let transport_mode = node
            .descendants()
            .find(|node| node.tag_name().name() == "TransportMode")
            .unwrap()
            .text()
            .unwrap_or_default();
        let mut result = ServiceJourney {
            day_type: day_type.to_owned(),
            transport_mode: transport_mode.to_owned(),
            ..ServiceJourney::default()
        };
        let passing_times_node = node
            .descendants()
            .find(|node| node.tag_name().name() == "passingTimes")
            .expect("ServiceJouney tag has no passing times");
        for timetabled in passing_times_node
            .descendants()
            .filter(|node| node.tag_name().name() == "TimetabledPassingTime")
        {
            let mut timetabled_passing_time = TimetabledPassingTime::default();
            for child in timetabled.descendants() {
                match child.tag_name().name() {
                    "StopPointInJourneyPatternRef" => {
                        timetabled_passing_time.stop_point_in_journey_pattern =
                            child.attribute("ref").unwrap_or_default().to_owned();
                    }
                    "ArrivalTime" => {
                        timetabled_passing_time.arrival =
                            Self::parse_minutes(child.text().unwrap_or_default());
                    }
                    "DepartureTime" => {
                        timetabled_passing_time.departure =
                            Self::parse_minutes(child.text().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            result.passing_times.push(timetabled_passing_time);
        }
        result
    }

    fn parse_operating_period(node: &roxmltree::Node) -> UicOperatingPeriod {
        let mut result = UicOperatingPeriod {
            id: node.attribute("id").unwrap_or_default().to_owned(),
            ..UicOperatingPeriod::default()
        };
        for child in node.descendants() {
            match child.tag_name().name() {
                "FromDate" => result.from = Self::parse_date(child.text().unwrap_or_default()),
                "ToDate" => result.to = Self::parse_date(child.text().unwrap_or_default()),
                "ValidDayBits" => {
                    result.valid_day_bits =
                        Self::parse_day_bits(child.text().unwrap_or_default().to_owned())
                }
                _ => {}
            }
        }
        return result;
    }

    fn parse_day_type_assignment(
        node: &roxmltree::Node,
    ) -> Result<DayTypeAssignment, Box<dyn std::error::Error>> {
        let mut assignment = DayTypeAssignment::default();
        for child in node.descendants() {
            match child.tag_name().name() {
                "OperatingPeriodRef" => {
                    assignment.operating_period =
                        child.attribute("ref").unwrap_or_default().to_owned()
                }
                "DayTypeRef" => {
                    assignment.day_type = child.attribute("ref").unwrap_or_default().to_owned()
                }
                "isAvailable" => {
                    assignment.is_available = child.text().unwrap_or_default().parse()?
                }
                _ => {}
            }
        }
        return Ok(assignment);
    }

    // In netex departure and arrival time are reqpresented as hh:mm:ss
    // seconds are mostly 00 anyway, so we only care about the minute of day
    // lets also assume times are represented as ascii chars
    fn parse_minutes(value: &str) -> u16 {
        const ASCII_ZERO: u16 = 48;
        let bytes = value.as_bytes();
        let mut result = 0_u16;
        result += (bytes[0] as u16 - ASCII_ZERO) * 600;
        result += (bytes[1] as u16 - ASCII_ZERO) * 60;
        result += (bytes[3] as u16 - ASCII_ZERO) * 10;
        result += bytes[4] as u16 - ASCII_ZERO;
        result
    }

    // Parses "2022-06-13T00:00:00" as 220613
    fn parse_date(value: &str) -> u32 {
        const ASCII_ZERO: u32 = 48;
        let bytes = value.as_bytes();
        let mut result = 0_u32;
        result += (bytes[2] as u32 - ASCII_ZERO) * 100000;
        result += (bytes[3] as u32 - ASCII_ZERO) * 10000;
        result += (bytes[5] as u32 - ASCII_ZERO) * 1000;
        result += (bytes[6] as u32 - ASCII_ZERO) * 100;
        result += (bytes[8] as u32 - ASCII_ZERO) * 10;
        result += bytes[9] as u32 - ASCII_ZERO;
        result
    }

    // Parses "11001100"... as Vec<u8>
    fn parse_day_bits(mut value: String) -> Vec<u8> {
        let pad_len = 8 - (value.as_bytes().len() % 8);
        if pad_len != 8 {
            value.push_str(&"0".repeat(pad_len));
        }
        let mut result = Vec::<u8>::with_capacity(value.len() / 8);
        for group in value.as_bytes().chunks(8) {
            result.push(Self::parse_day_bit_group(group))
        }
        result
    }

    // value should be at least 8 byte long
    fn parse_day_bit_group(value: &[u8]) -> u8 {
        const ASCII_ZERO: u8 = 48;
        let mut result = 0_u8;
        for i in 0..8 {
            result |= (value[i] - ASCII_ZERO) << i;
        }
        result
    }
}

mod tests {
    #[test]
    fn parse_minutes() {
        let result = super::NetexData::parse_minutes("12:34");
        assert_eq!(result, 754);
    }

    #[test]
    fn parse_day_bits_group() {
        let result = super::NetexData::parse_day_bits("1111111011".to_owned());
        assert_eq!(&result, &[127, 3]);
    }

    #[test]
    fn parse_date() {
        let result = super::NetexData::parse_date("2022-06-13T00:00:00");
        assert_eq!(result, 220613);
    }
}
