use schrodinger_plugin_sdk::prelude::*;
use schrodinger_plugin_sdk::schrodinger_plugin;

mod code_layout;

#[derive(Default)]
pub struct OcrPlugin;

impl NativePlugin for OcrPlugin {
    fn analyze(&self, entry: &ClipEntry) -> Result<AnalysisData, PluginError> {
        if !entry.has_image_format() {
            return Ok(AnalysisData::empty());
        }

        let image_data = entry.read_image()?;
        let tmp = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .map_err(|e| PluginError(format!("tempfile: {}", e)))?;
        std::fs::write(tmp.path(), &image_data)
            .map_err(|e| PluginError(format!("write: {}", e)))?;

        let results = loupe_rs::recognize_text(tmp.path())
            .map_err(|e| PluginError(e))?;

        Ok(AnalysisData::new().set("has_text", !results.is_empty()))
    }

    fn applies_to(&self, entry: &ClipEntry) -> bool {
        entry.has_image_format()
            && entry.analysis().has("has_text")
    }

    fn apply(
        &self,
        entry: &ClipEntry,
        mode: Option<&str>,
    ) -> Result<Vec<Output>, PluginError> {
        let image_data = entry.read_image()?;
        let tmp = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .map_err(|e| PluginError(format!("tempfile: {}", e)))?;
        std::fs::write(tmp.path(), &image_data)
            .map_err(|e| PluginError(format!("write: {}", e)))?;

        // Code wants the characters exactly as read; prose reads better with
        // the engine's dictionary correction.
        let as_code = mode == Some("code");
        let options = loupe_rs::OcrOptions { language_correction: !as_code };
        let results = loupe_rs::recognize_text_with(tmp.path(), options)
            .map_err(|e| PluginError(e))?;

        let text = if as_code {
            let lines: Vec<code_layout::Line> = results
                .iter()
                .map(|r| code_layout::Line {
                    text: r.text.clone(),
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                })
                .collect();
            code_layout::rebuild(&lines)
        } else {
            results.iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };

        Ok(vec![Output::text(&text)])
    }
}

schrodinger_plugin!(OcrPlugin);
