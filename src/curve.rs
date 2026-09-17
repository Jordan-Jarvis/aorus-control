//! Fan-curve validation and wire encoding.
//!
//! Firmware fan levels are not required to be monotonic. Some supported
//! laptops expose an OEM curve with a fan-level dip, so validation constrains
//! the temperature axis and raw value range without rewriting the curve.

use serde::{Deserialize, Serialize};
use std::{fmt, ops::Index};

pub const FAN_CURVE_POINTS: usize = 15;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FanPoint {
    pub temperature: u8,
    pub raw_speed: u8,
}

impl FanPoint {
    pub const fn new(temperature: u8, raw_speed: u8) -> Self {
        Self {
            temperature,
            raw_speed,
        }
    }

    pub const fn packed(self) -> u16 {
        ((self.raw_speed as u16) << 8) | self.temperature as u16
    }

    pub const fn encode(self) -> u16 {
        self.packed()
    }

    pub fn from_packed(packed: u16) -> Self {
        Self::new(packed as u8, (packed >> 8) as u8)
    }

    pub const fn decode(packed: u16) -> Self {
        Self {
            temperature: packed as u8,
            raw_speed: (packed >> 8) as u8,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FanCurve {
    points: [FanPoint; FAN_CURVE_POINTS],
}

impl Serialize for FanCurve {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.points
            .map(|point| [point.temperature, point.raw_speed])
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FanCurve {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let points = <Vec<[u8; 2]>>::deserialize(deserializer)?
            .into_iter()
            .map(|[temperature, raw_speed]| FanPoint::new(temperature, raw_speed))
            .collect();
        Self::new(points).map_err(serde::de::Error::custom)
    }
}

impl FanCurve {
    pub fn new(points: Vec<FanPoint>) -> Result<Self, CurveError> {
        let points: [FanPoint; FAN_CURVE_POINTS] = points
            .try_into()
            .map_err(|points: Vec<FanPoint>| CurveError::PointCount(points.len()))?;
        Self::from_array(points)
    }

    pub fn from_array(points: [FanPoint; FAN_CURVE_POINTS]) -> Result<Self, CurveError> {
        let curve = Self { points };
        curve.validate()?;
        Ok(curve)
    }

    pub const fn from_valid_array(points: [FanPoint; FAN_CURVE_POINTS]) -> Self {
        Self { points }
    }

    pub fn validate(&self) -> Result<(), CurveError> {
        for (index, point) in self.points.iter().enumerate() {
            if point.temperature > 100 {
                return Err(CurveError::Temperature {
                    index,
                    value: point.temperature,
                });
            }
            if index > 0 {
                let previous = self.points[index - 1];
                if point.temperature < previous.temperature {
                    return Err(CurveError::TemperatureOrder { index });
                }
            }
        }
        Ok(())
    }

    pub const fn points(&self) -> &[FanPoint; FAN_CURVE_POINTS] {
        &self.points
    }

    pub fn into_points(self) -> [FanPoint; FAN_CURVE_POINTS] {
        self.points
    }

    pub fn point(&self, index: usize) -> Option<FanPoint> {
        self.points.get(index).copied()
    }

    pub fn packed_points(&self) -> [u16; FAN_CURVE_POINTS] {
        std::array::from_fn(|index| self.points[index].packed())
    }

    pub fn clamped_point(
        &self,
        index: usize,
        temperature: u8,
        raw_speed: u8,
    ) -> Result<FanPoint, CurveError> {
        self.validate()?;
        self.points.get(index).ok_or(CurveError::Index(index))?;
        let previous = index.checked_sub(1).and_then(|i| self.points.get(i));
        let next = self.points.get(index + 1);
        let min_temperature = previous.map_or(0, |point| point.temperature);
        let max_temperature = next.map_or(100, |point| point.temperature);
        Ok(FanPoint::new(
            temperature.clamp(min_temperature, max_temperature),
            raw_speed,
        ))
    }

    pub fn with_clamped_point(
        &self,
        index: usize,
        temperature: u8,
        raw_speed: u8,
    ) -> Result<Self, CurveError> {
        let point = self.clamped_point(index, temperature, raw_speed)?;
        let mut points = self.points;
        *points.get_mut(index).ok_or(CurveError::Index(index))? = point;
        Self::from_array(points)
    }
}

impl Index<usize> for FanCurve {
    type Output = FanPoint;

    fn index(&self, index: usize) -> &Self::Output {
        &self.points[index]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CurveError {
    PointCount(usize),
    Index(usize),
    Temperature { index: usize, value: u8 },
    TemperatureOrder { index: usize },
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PointCount(count) => write!(
                f,
                "fan curve requires exactly {FAN_CURVE_POINTS} points, got {count}"
            ),
            Self::Index(index) => write!(f, "fan curve point index {index} is out of range"),
            Self::Temperature { index, value } => {
                write!(f, "point {index} temperature {value} is above 100°C")
            }
            Self::TemperatureOrder { index } => write!(f, "point {index} temperature decreases"),
        }
    }
}

impl std::error::Error for CurveError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> FanCurve {
        FanCurve::from_valid_array(std::array::from_fn(|index| {
            FanPoint::new(index as u8 * 5, index as u8 * 10)
        }))
    }

    #[test]
    fn packed_encoding_is_little_field_first() {
        let point = FanPoint::new(57, 200);
        assert_eq!(point.packed(), 0xc839);
        assert_eq!(FanPoint::from_packed(point.packed()), point);
    }

    #[test]
    fn validation_rejects_count_range_and_temperature_order() {
        assert!(matches!(
            FanCurve::new(vec![]),
            Err(CurveError::PointCount(0))
        ));
        let mut points = curve().into_points();
        points[4].temperature = 101;
        assert!(matches!(
            FanCurve::from_array(points),
            Err(CurveError::Temperature { index: 4, .. })
        ));
        let mut points = curve().into_points();
        points[4].temperature = points[3].temperature - 1;
        assert!(matches!(
            FanCurve::from_array(points),
            Err(CurveError::TemperatureOrder { index: 4 })
        ));
        let mut points = curve().into_points();
        points[4].raw_speed = points[3].raw_speed - 1;
        assert!(FanCurve::from_array(points).is_ok());
    }

    #[test]
    fn clamping_keeps_temperature_points_ordered() {
        let curve = curve();
        let changed = curve.with_clamped_point(5, 0, 0).unwrap();
        assert_eq!(changed[5], FanPoint::new(20, 0));
        let changed = curve.with_clamped_point(14, 100, 255).unwrap();
        assert_eq!(changed[14], FanPoint::new(100, 255));
        assert!(changed.validate().is_ok());
    }

    #[test]
    fn serde_round_trip_preserves_all_fifteen_points() {
        #[derive(Deserialize, Serialize)]
        struct Wrapper {
            curve: FanCurve,
        }
        let source = curve();
        let text = toml::to_string(&Wrapper {
            curve: source.clone(),
        })
        .unwrap();
        let restored: Wrapper = toml::from_str(&text).unwrap();
        assert_eq!(source, restored.curve);
    }
}
