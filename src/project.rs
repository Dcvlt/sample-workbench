use crate::edits::{EditHistory, EditState};
use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

pub(crate) struct ProjectData {
    pub source: PathBuf,
    pub state: EditState,
    pub selection: Option<Range<usize>>,
    pub gain: f32,
    pub looping: bool,
    pub bypass: bool,
    pub markers: Vec<usize>,
}

fn ranges(ranges: &[Range<usize>]) -> String {
    ranges
        .iter()
        .map(|r| format!("{}:{}", r.start, r.end))
        .collect::<Vec<_>>()
        .join(",")
}
fn parse_ranges(value: &str) -> Result<Vec<Range<usize>>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|part| {
            let (a, b) = part.split_once(':').ok_or("invalid range")?;
            Ok(a.parse().map_err(|_| "invalid range start")?
                ..b.parse().map_err(|_| "invalid range end")?)
        })
        .collect()
}

pub(crate) fn save(path: &Path, data: &ProjectData) -> Result<(), String> {
    let selection = data
        .selection
        .as_ref()
        .map_or(String::new(), |r| format!("{}:{}", r.start, r.end));
    let mut text = format!(
        "SAMPLE_WORKBENCH_PROJECT=1\nsource={}\ngain={:.9}\nlooping={}\nbypass={}\nfade_frames={}\nsilences={}\ndeletions={}\nselection={}\n",
        data.source.display(),
        data.gain,
        data.looping,
        data.bypass,
        data.state.fade_frames,
        ranges(&data.state.silences),
        ranges(&data.state.deletions),
        selection
    );
    text.push_str(&format!(
        "markers={}\n",
        data.markers
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ));
    fs::write(path, text).map_err(|e| format!("Could not save project: {e}"))
}

pub(crate) fn load(path: &Path) -> Result<ProjectData, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Could not open project: {e}"))?;
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            values.insert(k, v);
        }
    }
    if values.get("SAMPLE_WORKBENCH_PROJECT").copied() != Some("1") {
        return Err("Not a Sample Workbench project".into());
    }
    let selection = values
        .get("selection")
        .filter(|v| !v.is_empty())
        .map(|v| {
            parse_ranges(v).and_then(|mut r| r.pop().ok_or_else(|| "invalid selection".to_string()))
        })
        .transpose()?;
    Ok(ProjectData {
        source: PathBuf::from(*values.get("source").ok_or("project has no source")?),
        gain: values
            .get("gain")
            .ok_or("project has no gain")?
            .parse()
            .map_err(|_| "invalid gain")?,
        looping: values
            .get("looping")
            .unwrap_or(&"false")
            .parse()
            .map_err(|_| "invalid looping")?,
        bypass: values
            .get("bypass")
            .unwrap_or(&"false")
            .parse()
            .map_err(|_| "invalid bypass")?,
        state: EditState {
            fade_frames: values
                .get("fade_frames")
                .unwrap_or(&"0")
                .parse()
                .map_err(|_| "invalid fade")?,
            silences: parse_ranges(values.get("silences").unwrap_or(&""))?,
            deletions: parse_ranges(values.get("deletions").unwrap_or(&""))?,
        },
        selection,
        markers: values
            .get("markers")
            .filter(|v| !v.is_empty())
            .map(|v| {
                v.split(',')
                    .map(|n| {
                        n.parse::<usize>()
                            .map_err(|_| "invalid attack marker".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default(),
    })
}

pub(crate) fn history(state: EditState) -> EditHistory {
    let mut h = EditHistory::default();
    h.current = state;
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_attack_markers_round_trip_with_project() {
        let path = std::env::temp_dir().join(format!(
            "workbench-markers-{}-{}.swp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data = ProjectData {
            source: PathBuf::from("test.wav"),
            state: EditState::default(),
            selection: Some(4..9),
            gain: 0.5,
            looping: true,
            bypass: false,
            markers: vec![4, 8, 12],
        };
        save(&path, &data).unwrap();
        let reopened = load(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(reopened.markers, data.markers);
        assert_eq!(reopened.selection, data.selection);
        assert_eq!(reopened.gain, 0.5);
    }
    #[test]
    fn ranges_round_trip() {
        let v = vec![1..4, 8..9];
        assert_eq!(parse_ranges(&ranges(&v)).unwrap(), v);
    }
}
