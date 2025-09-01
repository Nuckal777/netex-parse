use xxhash_rust::xxh3::xxh3_64;

#[derive(Clone, Default)]
pub struct Authority {
    pub id: u64,
    pub short_name: String,
}

#[derive(Clone, Default)]
pub struct Line {
    pub id: u64,
    pub short_name: String,
    pub authority: u64,
}

#[derive(Clone, Default)]
pub struct DayTypeAssignment {
    pub operating_period: u64,
    pub day_type: u64,
    pub is_available: bool,
}

#[derive(Clone, Default)]
pub struct UicOperatingPeriod {
    pub id: u64,
    pub from: u16,
    pub to: u16,
    pub valid_day_bits: Vec<u8>,
}

#[derive(Default)]
pub struct ScheduledStopPoint {
    pub id: u64,
    pub short_name: String,
    pub long: f32,
    pub lat: f32,
}

#[derive(Default)]
pub struct StopPointInJourneyPattern {
    pub id: u64,
    pub scheduled_stop_point: u64,
}

#[derive(Default)]
pub struct ServiceJourneyPattern {
    pub stops: Vec<StopPointInJourneyPattern>,
    pub line: u64,
    pub id: u64,
}

#[derive(Default)]
pub struct TimetabledPassingTime {
    pub stop_point_in_journey_pattern: u64,
    pub arrival: u16,
    pub departure: u16,
}

#[derive(Default)]
pub struct ServiceJourney {
    pub passing_times: Vec<TimetabledPassingTime>,
    pub day_type: u64,
    pub transport_mode: String,
    pub pattern_ref: u64,
}

#[derive(Default)]
pub struct NetexData {
    pub scheduled_stop_points: Vec<ScheduledStopPoint>,
    pub service_journey_patterns: Vec<ServiceJourneyPattern>,
    pub service_journeys: Vec<ServiceJourney>,
    pub operating_periods: Vec<UicOperatingPeriod>,
    pub day_type_assignments: Vec<DayTypeAssignment>,
    pub lines: Vec<Line>,
    pub authorities: Vec<Authority>,
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

        let points: Vec<ServiceJourneyPattern> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "ServiceJourneyPattern")
            .map(|node| NetexData::parse_service_journey_pattern(&node))
            .collect();
        data.service_journey_patterns = points;

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

        let lines: Vec<Line> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "Line")
            .map(|node| NetexData::parse_line(&node))
            .collect();
        data.lines = lines;

        let authorities: Vec<Authority> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "Authority")
            .map(|node| NetexData::parse_authority(&node))
            .collect();
        data.authorities = authorities;
        Ok(data)
    }

    fn parse_scheduled_stop_point(
        node: &roxmltree::Node,
    ) -> Result<ScheduledStopPoint, Box<dyn std::error::Error>> {
        let mut result = ScheduledStopPoint {
            id: xxh3_64(node.attribute("id").unwrap_or_default().as_bytes()),
            ..ScheduledStopPoint::default()
        };
        for child in node.descendants() {
            match child.tag_name().name() {
                "ShortName" | "Name" => {
                    result.short_name = child.text().unwrap_or_default().replace('"', "");
                }
                "Longitude" => {
                    result.long = child
                        .text()
                        .unwrap_or_default()
                        .parse::<f32>()?
                        .clamp(-180.0, 180.0);
                }
                "Latitude" => {
                    result.lat = child
                        .text()
                        .unwrap_or_default()
                        .parse::<f32>()?
                        .clamp(-90.0, 90.0);
                }
                _ => {}
            }
        }
        Ok(result)
    }

    fn parse_service_journey_pattern(node: &roxmltree::Node) -> ServiceJourneyPattern {
        let mut result = ServiceJourneyPattern {
            id: xxh3_64(node.attribute("id").unwrap_or_default().as_bytes()),
            ..ServiceJourneyPattern::default()
        };
        for sub_node in node.descendants() {
            if sub_node.tag_name().name() == "LineRef" {
                result.line = xxh3_64(sub_node.attribute("ref").unwrap_or_default().as_bytes());
            }
            if sub_node.tag_name().name() != "StopPointInJourneyPattern" {
                continue;
            }
            let mut stop = StopPointInJourneyPattern {
                id: xxh3_64(sub_node.attribute("id").unwrap_or_default().as_bytes()),
                ..StopPointInJourneyPattern::default()
            };
            stop.scheduled_stop_point = xxh3_64(
                sub_node
                    .descendants()
                    .find(|child| child.tag_name().name() == "ScheduledStopPointRef")
                    .map(|node| node.attribute("ref").unwrap_or_default().as_bytes())
                    .unwrap_or_default(),
            );
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
        let pattern_ref = node
            .descendants()
            .find(|node| node.tag_name().name() == "ServiceJourneyPatternRef")
            .unwrap()
            .attribute("ref")
            .unwrap_or_default();
        let mut result = ServiceJourney {
            day_type: xxh3_64(day_type.as_bytes()),
            transport_mode: transport_mode.to_owned(),
            pattern_ref: xxh3_64(pattern_ref.as_bytes()),
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
                            xxh3_64(child.attribute("ref").unwrap_or_default().as_bytes());
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
            id: xxh3_64(node.attribute("id").unwrap_or_default().as_bytes()),
            ..UicOperatingPeriod::default()
        };
        for child in node.descendants() {
            match child.tag_name().name() {
                "FromDate" => result.from = Self::parse_date(child.text().unwrap_or_default()),
                "ToDate" => result.to = Self::parse_date(child.text().unwrap_or_default()),
                "ValidDayBits" => {
                    result.valid_day_bits =
                        Self::parse_day_bits(child.text().unwrap_or_default().to_owned());
                }
                _ => {}
            }
        }
        result
    }

    fn parse_day_type_assignment(
        node: &roxmltree::Node,
    ) -> Result<DayTypeAssignment, Box<dyn std::error::Error>> {
        let mut assignment = DayTypeAssignment::default();
        for child in node.descendants() {
            match child.tag_name().name() {
                "OperatingPeriodRef" => {
                    assignment.operating_period =
                        xxh3_64(child.attribute("ref").unwrap_or_default().as_bytes());
                }
                "DayTypeRef" => {
                    assignment.day_type =
                        xxh3_64(child.attribute("ref").unwrap_or_default().as_bytes());
                }
                "isAvailable" => {
                    assignment.is_available = child.text().unwrap_or_default().parse()?;
                }
                _ => {}
            }
        }
        Ok(assignment)
    }

    fn parse_line(node: &roxmltree::Node) -> Line {
        let mut result = Line {
            id: xxh3_64(node.attribute("id").unwrap_or_default().as_bytes()),
            ..Line::default()
        };
        for child in node.descendants() {
            match child.tag_name().name() {
                "ShortName" => {
                    child.text().unwrap_or_default().clone_into(&mut result.short_name);
                }
                "AuthorityRef" => {
                    result.authority =
                        xxh3_64(child.attribute("ref").unwrap_or_default().as_bytes());
                }
                _ => {}
            }
        }
        result
    }

    fn parse_authority(node: &roxmltree::Node) -> Authority {
        let mut result = Authority {
            id: xxh3_64(node.attribute("id").unwrap_or_default().as_bytes()),
            ..Authority::default()
        };
        for child in node.descendants() {
            if child.tag_name().name() == "ShortName" {
                child.text().unwrap_or_default().clone_into(&mut result.short_name);
            }
        }
        result
    }

    // In netex departure and arrival time are reqpresented as hh:mm:ss
    // seconds are mostly 00 anyway, so we only care about the minute of day
    // lets also assume times are represented as ascii chars
    #[allow(clippy::cast_lossless)]
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

    // Parses "2022-06-13T00:00:00" into bits
    // day 5 bit, month 4 bit, year 7 bit
    #[allow(clippy::cast_lossless)]
    fn parse_date(value: &str) -> u16 {
        const ASCII_ZERO: u16 = 48;
        let bytes = value.as_bytes();
        // year
        let mut year: u16 = (bytes[2] as u16 - ASCII_ZERO) * 10;
        assert!(year < 128);
        year += bytes[3] as u16 - ASCII_ZERO;
        // month
        let mut month: u16 = (bytes[5] as u16 - ASCII_ZERO) * 10;
        assert!(month < 16);
        month += bytes[6] as u16 - ASCII_ZERO;
        year |= month << 7;
        // day
        let mut day: u16 = (bytes[8] as u16 - ASCII_ZERO) * 10;
        assert!(day < 32);
        day += bytes[9] as u16 - ASCII_ZERO;
        year | (day << 11)
    }

    // Parses "11001100"... as Vec<u8>
    fn parse_day_bits(mut value: String) -> Vec<u8> {
        let pad_len = 8 - (value.len() % 8);
        if pad_len != 8 {
            value.push_str(&"0".repeat(pad_len));
        }
        let mut result = Vec::<u8>::with_capacity(value.len() / 8);
        for group in value.as_bytes().chunks(8) {
            result.push(Self::parse_day_bit_group(group));
        }
        result
    }

    // value should be at least 8 byte long
    #[allow(clippy::needless_range_loop)]
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
        let result = super::NetexData::parse_date("2003-02-01T00:00:00");
        assert_eq!(result, 0b_0000_1001_0000_0011);
    }

    #[test]
    fn decode_date() {
        let parsed = super::NetexData::parse_date("2045-11-23T00:00:00");
        let year = parsed & 0b_0000_0000_0111_1111;
        let month = (parsed >> 7) & 0b_0000_0000_0000_1111;
        let day = (parsed >> 11) & 0b_0000_0000_0001_1111;
        assert_eq!(year, 45_u16);
        assert_eq!(month, 11_u16);
        assert_eq!(day, 23_u16);
    }
}
