//! Explicit RAW-smart import. Unsupported sensor data is an error, never a JPEG fallback.
use crate::{ImportResult, IoError};
use photocraft_doc::{LayerContent, RawDemosaic, RawDevelopment, RawWhiteBalance, SmartObject, SmartSource};
use photocraft_raw::{Demosaic, DevelopOptions, Sensor, WhiteBalance};
use std::sync::{Arc, Mutex};

type SensorEntry = ([u8; 32], Arc<Sensor>);
static SENSORS: Mutex<Vec<SensorEntry>> = Mutex::new(Vec::new());
const MAX_SENSOR_BYTES: usize = 256 * 1024 * 1024;

fn sensor_bytes(s: &Sensor) -> usize {
    let mut bytes = s.data.len().saturating_mul(2);
    bytes = bytes.saturating_add(s.linearization.as_ref().map_or(0, |v| v.len().saturating_mul(2)));
    for v in [&s.black.values, &s.black.delta_h, &s.black.delta_v] {
        bytes = bytes.saturating_add(v.len().saturating_mul(4));
    }
    for map in &s.gain_maps {
        bytes = bytes.saturating_add(map.gains.len().saturating_mul(4));
    }
    bytes = bytes.saturating_add(s.cfa.as_ref().map_or(0, |c| c.colors.len()));
    bytes = bytes.saturating_add(s.make.as_ref().map_or(0, String::len)).saturating_add(s.model.as_ref().map_or(0, String::len));
    bytes = bytes.saturating_add(s.color.calibrations.len().saturating_mul(std::mem::size_of::<photocraft_raw::Calibration>()));
    s.warnings.iter().fold(bytes, |bytes, warning| bytes.saturating_add(warning.len()))
}

pub fn develop(name: &str, bytes: &[u8], settings: &RawDevelopment) -> Result<ImportResult, IoError> {
    settings.validate().map_err(|e| IoError::Raw(photocraft_raw::RawError::Malformed(e)))?;
    let sensor = sensor(bytes)?;
    develop_sensor(name, &sensor, settings)
}

fn sensor(bytes: &[u8]) -> Result<Arc<Sensor>, IoError> {
    let key = *blake3::hash(bytes).as_bytes();
    let cached = SENSORS.lock().ok().and_then(|mut cache| {
        let i = cache.iter().position(|(k, _)| *k == key)?;
        let entry = cache.remove(i);
        let sensor = entry.1.clone();
        cache.push(entry);
        Some(sensor)
    });
    let sensor = match cached {
        Some(sensor) => sensor,
        None => {
            let sensor = Arc::new(photocraft_raw::decode(bytes, &Default::default())?);
            if sensor_bytes(&sensor) <= MAX_SENSOR_BYTES
                && let Ok(mut cache) = SENSORS.lock()
            {
                cache.retain(|(k, _)| *k != key);
                cache.push((key, sensor.clone()));
                while cache.len() > 2 || cache.iter().fold(0usize, |bytes, (_, s)| bytes.saturating_add(sensor_bytes(s))) > MAX_SENSOR_BYTES {
                    cache.remove(0);
                }
            }
            sensor
        }
    };
    Ok(sensor)
}

fn develop_sensor(name: &str, sensor: &Sensor, settings: &RawDevelopment) -> Result<ImportResult, IoError> {
    let opts = DevelopOptions {
        exposure: settings.exposure,
        white_balance: match settings.white_balance {
            RawWhiteBalance::AsShot => WhiteBalance::AsShot,
            RawWhiteBalance::Auto => WhiteBalance::Auto,
            RawWhiteBalance::Multipliers { rgb } => WhiteBalance::Multipliers(rgb),
        },
        demosaic: match settings.demosaic {
            RawDemosaic::Bilinear => Demosaic::Bilinear,
            RawDemosaic::Mhc => Demosaic::Mhc,
            RawDemosaic::Ahd => Demosaic::Ahd,
        },
        ..Default::default()
    };
    let dev = photocraft_raw::develop_sensor(sensor, &opts)?;
    let image = photocraft_codecs::Image::from_u16(dev.width, dev.height, photocraft_codecs::ChannelLayout::Rgb, &dev.rgb)?;
    let mut imported = crate::flat::image_to_document(name, &image)?;
    imported.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    imported.warnings.extend(dev.warnings);
    Ok(imported)
}

pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_with_sensor(name, bytes, None)
}

/// Sensor adapter is explicit: no fallback to a camera JPEG or developed RGB image.
pub fn import_with_sensor(name: &str, bytes: &[u8], sensor: Option<(&str, &[u8])>) -> Result<ImportResult, IoError> {
    let settings = RawDevelopment::default();
    let (decode_name, decode_bytes) = sensor.unwrap_or((name, bytes));
    let decoded = self::sensor(decode_bytes)?;
    if sensor.is_some() && (decoded.format != photocraft_raw::RawFormat::Dng || decoded.cfa.is_none()) {
        return Err(IoError::Raw(photocraft_raw::RawError::Malformed("RAW adapter requires a Bayer sensor DNG, never developed RGB".into())));
    }
    let mut imported = develop_sensor(decode_name, &decoded, &settings)?;
    imported.document.name = name.into();
    let layer = imported.document.layers.first_mut().ok_or_else(|| IoError::Raw(photocraft_raw::RawError::Malformed("RAW import produced no layer".into())))?;
    let LayerContent::Raster(surface) = &layer.content else {
        return Err(IoError::Raw(photocraft_raw::RawError::Malformed("RAW import produced no pixels".into())));
    };
    let mut smart = SmartObject::new(
        SmartSource::Embedded { file_name: name.into(), bytes: Arc::new(bytes.to_vec()) },
        photocraft_geom::Affine::IDENTITY,
        Some(surface.clone()),
    );
    smart.raw_development = Some(settings);
    smart.raw_decode_source = sensor.map(|(file_name, bytes)| SmartSource::Embedded { file_name: file_name.into(), bytes: Arc::new(bytes.to_vec()) });
    layer.content = LayerContent::Smart(smart);
    layer.name = name.into();
    layer.locks = Default::default();
    Ok(imported)
}
