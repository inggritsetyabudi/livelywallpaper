// The property model in this module (`PropertyValue`, `LivelyPropertyItem`,
// `LivelyProperties`) exists for the settings UI, which is not built yet. It is
// parsed and unit tested so the manifest format stays pinned, and is therefore
// reported as dead code until a non-test consumer lands.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivelyInfo {
    #[serde(rename = "AppVersion", default = "default_version")]
    pub app_version: String,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Thumbnail", default)]
    pub thumbnail: Option<String>,
    #[serde(rename = "Preview", default)]
    pub preview: Option<String>,
    #[serde(rename = "Desc", default)]
    pub desc: Option<String>,
    #[serde(rename = "Author", default)]
    pub author: Option<String>,
    #[serde(rename = "License", default)]
    pub license: Option<String>,
    #[serde(rename = "Contact", default)]
    pub contact: Option<String>,
    #[serde(rename = "Type")]
    pub wallpaper_type: i32, // 0=Video, 1=Web, 2=WebGL, 3=App, 4=Gif, 5=Picture, 6=Shader
    #[serde(rename = "FileName")]
    pub file_name: String,
    #[serde(rename = "Arguments", default)]
    pub arguments: Option<String>,
    #[serde(skip)]
    pub root_folder: PathBuf,
}

fn default_version() -> String {
    "2.0.0.0".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Number(f64),
    String(String),
    Bool(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivelyPropertyItem {
    #[serde(rename = "type")]
    pub prop_type: String, // slider, color, checkbox, dropdown, folderDropdown, button
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub value: Option<PropertyValue>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
}

pub type LivelyProperties = HashMap<String, LivelyPropertyItem>;

impl LivelyInfo {
    pub fn load_from_dir<P: AsRef<Path>>(dir: P) -> Result<Self, Box<dyn std::error::Error>> {
        let info_path = dir.as_ref().join("LivelyInfo.json");
        let content = std::fs::read_to_string(&info_path)?;
        let mut info: LivelyInfo = serde_json::from_str(&content)?;
        info.root_folder = dir.as_ref().to_path_buf();
        Ok(info)
    }

    pub fn target_path(&self) -> PathBuf {
        self.root_folder.join(&self.file_name)
    }

    /// Read `LivelyProperties.json`, the user-editable settings a wallpaper
    /// package can ship (sliders, colour pickers, dropdowns).
    ///
    /// Returns `None` when the file is absent — most packages have none — and
    /// also when it is present but unparseable: a broken settings file must not
    /// stop the wallpaper itself from loading.
    pub fn load_properties(&self) -> Option<LivelyProperties> {
        let prop_path = self.root_folder.join("LivelyProperties.json");
        if !prop_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&prop_path).ok()?;
        match serde_json::from_str::<LivelyProperties>(&content) {
            Ok(props) => Some(props),
            Err(e) => {
                log::warn!("ignoring malformed {prop_path:?}: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lively-pkg-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_package(dir: &Path, info: &str) {
        fs::write(dir.join("LivelyInfo.json"), info).unwrap();
    }

    #[test]
    fn loads_a_minimal_info_file() {
        let dir = tmpdir("minimal");
        write_package(&dir, r#"{"Title":"Demo","Type":0,"FileName":"a.mp4"}"#);

        let info = LivelyInfo::load_from_dir(&dir).unwrap();
        assert_eq!(info.title, "Demo");
        assert_eq!(info.wallpaper_type, 0);
        assert_eq!(info.file_name, "a.mp4");
        assert_eq!(info.root_folder, dir);
        assert_eq!(info.target_path(), dir.join("a.mp4"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let dir = tmpdir("defaults");
        write_package(&dir, r#"{"Title":"Bare","Type":5,"FileName":"a.png"}"#);

        let info = LivelyInfo::load_from_dir(&dir).unwrap();
        assert_eq!(info.app_version, "2.0.0.0", "AppVersion defaults");
        assert!(info.thumbnail.is_none());
        assert!(info.author.is_none());
        assert!(info.arguments.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_info_file_is_an_error() {
        let dir = tmpdir("noinfo");
        assert!(LivelyInfo::load_from_dir(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_info_file_is_an_error() {
        let dir = tmpdir("badinfo");
        write_package(&dir, "{ not json");
        assert!(LivelyInfo::load_from_dir(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn properties_are_absent_by_default() {
        let dir = tmpdir("noprops");
        write_package(&dir, r#"{"Title":"Demo","Type":0,"FileName":"a.mp4"}"#);
        let info = LivelyInfo::load_from_dir(&dir).unwrap();
        assert!(info.load_properties().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_slider_checkbox_and_dropdown_properties() {
        let dir = tmpdir("props");
        write_package(&dir, r#"{"Title":"Demo","Type":1,"FileName":"index.html"}"#);
        fs::write(
            dir.join("LivelyProperties.json"),
            r##"{
                "Brightness": {"type":"slider","text":"Brightness","value":80,"min":0,"max":100,"step":1},
                "Muted":      {"type":"checkbox","text":"Muted","value":false},
                "Theme":      {"type":"dropdown","text":"Theme","value":"dark","options":["dark","light"]},
                "Hue":        {"type":"color","text":"Hue","value":"#ff8800"},
                "Label":      {"type":"slider","text":"Label","value":"hello"}
            }"##,
        )
        .unwrap();

        let info = LivelyInfo::load_from_dir(&dir).unwrap();
        let props = info.load_properties().expect("properties should load");
        assert_eq!(props.len(), 5);

        assert_eq!(props["Brightness"].prop_type, "slider");
        assert_eq!(props["Brightness"].value, Some(PropertyValue::Number(80.0)));
        assert_eq!(props["Brightness"].min, Some(0.0));
        assert_eq!(props["Muted"].value, Some(PropertyValue::Bool(false)));
        assert_eq!(props["Theme"].options.as_ref().unwrap(), &["dark", "light"]);
        assert_eq!(
            props["Hue"].value,
            Some(PropertyValue::String("#ff8800".into()))
        );
        assert_eq!(
            props["Label"].value,
            Some(PropertyValue::String("hello".into()))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_properties_do_not_break_the_package() {
        let dir = tmpdir("badprops");
        write_package(&dir, r#"{"Title":"Demo","Type":0,"FileName":"a.mp4"}"#);
        fs::write(dir.join("LivelyProperties.json"), b"{ broken").unwrap();

        let info = LivelyInfo::load_from_dir(&dir).unwrap();
        assert!(
            info.load_properties().is_none(),
            "bad props must be ignored"
        );
        // Crucially, the package itself is still usable.
        assert_eq!(info.title, "Demo");

        let _ = fs::remove_dir_all(&dir);
    }
}
