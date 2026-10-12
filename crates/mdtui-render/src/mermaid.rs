//! Mermaid diagrams: `mmdc` → PNG (cached), falling back to `mermaid-ascii` text output.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{cache_dir, find_in_path, hash_str};

#[derive(Debug, Clone)]
pub struct MermaidConfig {
    /// Path or name of the mermaid-cli binary.
    pub mmdc: String,
    /// Path or name of the mermaid-ascii binary (text fallback).
    pub ascii: String,
    /// mermaid theme: default, dark, forest, neutral.
    pub theme: String,
    /// Background colour passed to mmdc (`transparent`, `white`, `#1e1e1e`, …).
    pub background: String,
    /// Optional puppeteer config file (e.g. to pass `--no-sandbox`).
    pub puppeteer_config: Option<String>,
    /// Render width in pixels.
    pub width: u32,
    /// If Chromium can't create its sandbox (e.g. Ubuntu 23.10+ AppArmor), retry with
    /// `--no-sandbox`. The page only renders your own diagram source.
    pub no_sandbox_fallback: bool,
}

impl Default for MermaidConfig {
    fn default() -> Self {
        MermaidConfig {
            mmdc: "mmdc".into(),
            ascii: "mermaid-ascii".into(),
            theme: "dark".into(),
            background: "transparent".into(),
            puppeteer_config: None,
            width: 1200,
            no_sandbox_fallback: true,
        }
    }
}

pub enum MermaidOutput {
    Png(Vec<u8>),
    Text(String),
}

pub fn render(src: &str, cfg: &MermaidConfig) -> Result<MermaidOutput, String> {
    let key = hash_str(&format!("{src}\0{}\0{}\0{}", cfg.theme, cfg.background, cfg.width));
    let dir = cache_dir("mermaid");
    let png = dir.join(format!("{key}.png"));
    if let Ok(b) = std::fs::read(&png) {
        return Ok(MermaidOutput::Png(b));
    }
    let input = dir.join(format!("{key}.mmd"));
    std::fs::write(&input, src).map_err(|e| e.to_string())?;

    let mut mmdc_err = None;
    if let Some(mmdc) = find_in_path(&cfg.mmdc) {
        // one Chromium at a time: they are heavy, and parallel launches race on first use
        static MMDC: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = MMDC.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(b) = std::fs::read(&png) {
            return Ok(MermaidOutput::Png(b));
        }
        let mut result = run_mmdc(&mmdc, &input, &png, cfg, None);
        if let Err(e) = &result {
            if e.contains("sandbox") && cfg.no_sandbox_fallback && cfg.puppeteer_config.is_none() {
                let pc = dir.join("puppeteer-no-sandbox.json");
                if !pc.exists() {
                    let tmp = dir.join(format!("puppeteer-{key}.tmp"));
                    let _ = std::fs::write(&tmp, r#"{"args":["--no-sandbox"]}"#);
                    let _ = std::fs::rename(&tmp, &pc);
                }
                result = run_mmdc(&mmdc, &input, &png, cfg, Some(&pc));
            }
        }
        match result {
            Ok(()) => {
                let _ = std::fs::remove_file(&input);
                return std::fs::read(&png)
                    .map(MermaidOutput::Png)
                    .map_err(|e| e.to_string());
            }
            Err(e) => mmdc_err = Some(e),
        }
    }
    if let Some(ascii) = find_in_path(&cfg.ascii) {
        let out = Command::new(ascii)
            .arg("-f")
            .arg(&input)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&input);
        if out.status.success() && !out.stdout.is_empty() {
            return Ok(MermaidOutput::Text(String::from_utf8_lossy(&out.stdout).into_owned()));
        }
        return Err(mmdc_err.unwrap_or_else(|| {
            format!("mermaid-ascii failed: {}", String::from_utf8_lossy(&out.stderr).trim())
        }));
    }
    let _ = std::fs::remove_file(&input);
    Err(mmdc_err.unwrap_or_else(|| {
        "install mermaid-cli (`npm i -g @mermaid-js/mermaid-cli`) or mermaid-ascii to render diagrams"
            .into()
    }))
}

/// mermaid-cli ≤ 11 takes `-w <px>`; 12+ replaced it with `--size <px>`. Ask `--help` once.
fn size_flag(mmdc: &PathBuf) -> &'static str {
    static FLAG: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| {
        let help = Command::new(mmdc)
            .arg("--help")
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        if help.contains("--size") {
            "--size"
        } else {
            "-w"
        }
    })
}

fn run_mmdc(
    mmdc: &PathBuf,
    input: &PathBuf,
    output: &PathBuf,
    cfg: &MermaidConfig,
    puppeteer_override: Option<&PathBuf>,
) -> Result<(), String> {
    let size_flag = size_flag(mmdc);
    let mut cmd = Command::new(mmdc);
    cmd.arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-t")
        .arg(&cfg.theme)
        .arg("-b")
        .arg(&cfg.background)
        .arg(size_flag)
        .arg(cfg.width.to_string())
        .arg("-q")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(p) = puppeteer_override {
        cmd.arg("-p").arg(p);
    } else if let Some(p) = &cfg.puppeteer_config {
        cmd.arg("-p").arg(p);
    }
    let out = cmd.output().map_err(|e| format!("mmdc: {e}"))?;
    if out.status.success() && output.exists() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .find(|l| l.contains("No usable sandbox"))
            .map(|_| "Chromium sandbox unavailable")
            .or_else(|| err.lines().find(|l| l.contains("Error") || l.contains("error")))
            .or_else(|| err.lines().find(|l| !l.trim().is_empty()))
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .unwrap_or("failed without output");
        Err(format!("mmdc: {}", line.trim_start_matches("Error:").trim()))
    }
}
