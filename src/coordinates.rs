use std::str::FromStr;

use nom::branch::alt;
use nom::character::complete::{char, digit1, satisfy, space1};
use nom::multi::count;
use nom::sequence::tuple;
use nom::{AsChar, IResult};
use serde::Serialize;

// north and east are positive numbers
#[derive(Debug, Copy, Clone, Serialize)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

pub fn coordinate_parser(i: &str) -> IResult<&str, Coordinates> {
    let (i, (lat_deg, lat_min, bearing, _)) = tuple((
        count(satisfy(|c| c.is_dec_digit()), 2),
        digit1,
        alt((char('N'), char('S'))),
        space1,
    ))(i)?;
    let lat_deg: String = lat_deg.iter().collect();
    let num = float_from_deg_min(&lat_deg, lat_min);
    let lat = match bearing {
        'S' => -num,
        _ => num,
    };
    let (i, (lon_deg, lon_min, bearing)) = tuple((
        count(satisfy(|c| c.is_dec_digit()), 3),
        digit1,
        alt((char('E'), char('W'))),
    ))(i)?;
    let lon_deg: String = lon_deg.iter().collect();
    let num = float_from_deg_min(&lon_deg, lon_min);
    let lon = match bearing {
        'W' => -num,
        _ => num,
    };
    Ok((i, Coordinates { lat, lon }))
}

fn float_from_deg_min(deg: &str, min: &str) -> f64 {
    f64::from_str(deg).unwrap() + f64::from_str(min).unwrap() / 60.0_f64
}
