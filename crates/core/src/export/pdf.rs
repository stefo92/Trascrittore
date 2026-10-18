//! Esportazione in PDF taggato e, quando possibile, conforme a PDF/UA-1.
//!
//! Il testo è vero testo (selezionabile e leggibile dagli screen reader), con struttura
//! logica (H1, H2, P), lingua, titolo nei metadati e numeri di pagina marcati come artefatti.

use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Timelike};
use krilla::configure::{Accessibility, ConfigurationBuilder};
use krilla::destination::XyzDestination;
use krilla::error::KrillaError;
use krilla::geom::Point;
use krilla::metadata::{DateTime, Metadata};
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::tagging::{
    Artifact, ArtifactType, ContentTag, Identifier, Node, SpanTag, Tag, TagGroup, TagTree,
};
use krilla::text::{Font, TextDirection};
use krilla::{Document as PdfDocument, SerializeSettings};

use crate::document::{Document, BODY_HEADING, INFO_HEADING};

/// Variabile d'ambiente con il percorso di un carattere TrueType da usare nel PDF.
pub const FONT_ENV: &str = "TRASCRITTORE_PDF_FONT";
/// Variabile d'ambiente con il percorso della variante grassetto.
pub const FONT_BOLD_ENV: &str = "TRASCRITTORE_PDF_FONT_BOLD";

// Pagina A4 in punti tipografici.
const PAGE_WIDTH: f32 = 595.28;
const PAGE_HEIGHT: f32 = 841.89;
const MARGIN: f32 = 56.69; // 2 cm
const FOOTER_SPACE: f32 = 24.0;
const FOOTER_SIZE: f32 = 9.0;
const BODY_SIZE: f32 = 11.5;
const BODY_LINE: f32 = 1.45;
const HEADING_LINE: f32 = 1.25;

/// Coppia di file di caratteri (normale e grassetto) trovati nel sistema.
#[derive(Debug, Clone)]
pub struct SystemFonts {
    pub regular: PathBuf,
    pub bold: PathBuf,
}

/// Cerca un carattere sans-serif con buona copertura dei caratteri latini.
///
/// Si può forzare la scelta con le variabili d'ambiente [`FONT_ENV`] e [`FONT_BOLD_ENV`].
pub fn find_system_fonts() -> Result<SystemFonts> {
    if let Some(regular) = std::env::var_os(FONT_ENV).map(PathBuf::from) {
        let bold = std::env::var_os(FONT_BOLD_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| regular.clone());
        return Ok(SystemFonts { regular, bold });
    }

    let mut candidates: Vec<(PathBuf, &str, &str)> = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let fonts = windir.join("Fonts");
        for (regular, bold) in [
            ("arial.ttf", "arialbd.ttf"),
            ("segoeui.ttf", "segoeuib.ttf"),
            ("calibri.ttf", "calibrib.ttf"),
            ("verdana.ttf", "verdanab.ttf"),
        ] {
            candidates.push((fonts.clone(), regular, bold));
        }
    } else if cfg!(target_os = "macos") {
        for dir in ["/System/Library/Fonts/Supplemental", "/Library/Fonts"] {
            candidates.push((PathBuf::from(dir), "Arial.ttf", "Arial Bold.ttf"));
        }
    } else {
        for dir in [
            "/usr/share/fonts/truetype/dejavu",
            "/usr/share/fonts/TTF",
            "/usr/share/fonts/dejavu",
            "/usr/share/fonts/dejavu-sans-fonts",
        ] {
            candidates.push((PathBuf::from(dir), "DejaVuSans.ttf", "DejaVuSans-Bold.ttf"));
        }
        for dir in [
            "/usr/share/fonts/truetype/liberation",
            "/usr/share/fonts/liberation-sans",
            "/usr/share/fonts/liberation",
        ] {
            candidates.push((
                PathBuf::from(dir),
                "LiberationSans-Regular.ttf",
                "LiberationSans-Bold.ttf",
            ));
        }
        for dir in ["/usr/share/fonts/truetype/noto", "/usr/share/fonts/noto"] {
            candidates.push((PathBuf::from(dir), "NotoSans-Regular.ttf", "NotoSans-Bold.ttf"));
        }
    }

    for (dir, regular, bold) in candidates {
        let regular = dir.join(regular);
        if regular.is_file() {
            let bold = dir.join(bold);
            let bold = if bold.is_file() { bold } else { regular.clone() };
            return Ok(SystemFonts { regular, bold });
        }
    }
    Err(anyhow!(
        "Nessun carattere adatto trovato per il PDF. Imposta la variabile d'ambiente {FONT_ENV} con il percorso di un file .ttf."
    ))
}

/// Carattere caricato, con le metriche necessarie all'impaginazione.
struct LoadedFont {
    data: Arc<Vec<u8>>,
    font: Font,
}

impl LoadedFont {
    fn load(path: &Path) -> Result<Self> {
        let data = Arc::new(
            std::fs::read(path)
                .with_context(|| format!("Impossibile leggere il carattere «{}».", path.display()))?,
        );
        let font = Font::new(Arc::clone(&data).into(), 0)
            .ok_or_else(|| anyhow!("Il file «{}» non è un carattere valido.", path.display()))?;
        Ok(Self { data, font })
    }

    fn metrics(&self) -> Result<Metrics<'_>> {
        let face = ttf_parser::Face::parse(&self.data, 0)
            .map_err(|e| anyhow!("Carattere non leggibile: {e}"))?;
        Ok(Metrics { face })
    }
}

struct Metrics<'a> {
    face: ttf_parser::Face<'a>,
}

impl Metrics<'_> {
    fn units_per_em(&self) -> f32 {
        f32::from(self.face.units_per_em())
    }

    fn width(&self, text: &str, size: f32) -> f32 {
        let fallback = self
            .face
            .glyph_hor_advance(ttf_parser::GlyphId(0))
            .unwrap_or(0);
        let units: u32 = text
            .chars()
            .map(|c| {
                self.face
                    .glyph_index(c)
                    .and_then(|g| self.face.glyph_hor_advance(g))
                    .unwrap_or(fallback) as u32
            })
            .sum();
        units as f32 * size / self.units_per_em()
    }

    fn ascent_ratio(&self) -> f32 {
        f32::from(self.face.ascender()) / self.units_per_em()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Heading(u16),
    Paragraph,
}

/// Blocco logico del documento (diventa un elemento della struttura del PDF).
struct Block {
    kind: BlockKind,
    text: String,
    bold: bool,
    size: f32,
    line_spacing: f32,
    space_before: f32,
    space_after: f32,
    lang: Option<String>,
}

/// Riga già impaginata.
struct Line {
    block: usize,
    text: String,
    x: f32,
    baseline: f32,
}

#[derive(Default)]
struct PageLayout {
    lines: Vec<Line>,
}

pub(crate) fn render(doc: &Document) -> Result<Vec<u8>> {
    let fonts = find_system_fonts()?;
    let regular = LoadedFont::load(&fonts.regular)?;
    let bold = if fonts.bold == fonts.regular {
        LoadedFont {
            data: Arc::clone(&regular.data),
            font: regular.font.clone(),
        }
    } else {
        LoadedFont::load(&fonts.bold)?
    };

    let blocks = build_blocks(doc);
    let pages = layout(&blocks, &regular.metrics()?, &bold.metrics()?);

    match write_pdf(doc, &blocks, &pages, &regular, &bold, true) {
        Ok(bytes) => Ok(bytes),
        // Ad esempio caratteri non presenti nel font: si produce comunque un PDF taggato,
        // senza dichiarare la conformità PDF/UA.
        Err(KrillaError::Validation(errors)) => {
            log::warn!(
                "PDF/UA non garantito ({} problemi di validazione): esporto un PDF taggato standard.",
                errors.len()
            );
            write_pdf(doc, &blocks, &pages, &regular, &bold, false)
                .map_err(|e| anyhow!("Errore nella creazione del PDF: {e}"))
        }
        Err(e) => Err(anyhow!("Errore nella creazione del PDF: {e}")),
    }
}

fn build_blocks(doc: &Document) -> Vec<Block> {
    let heading = |level: u16, text: &str| Block {
        kind: BlockKind::Heading(level),
        text: text.to_string(),
        bold: true,
        size: if level == 1 { 20.0 } else { 14.5 },
        line_spacing: HEADING_LINE,
        space_before: if level == 1 { 0.0 } else { 16.0 },
        space_after: 8.0,
        lang: None,
    };
    let paragraph = |text: String, lang: Option<String>| Block {
        kind: BlockKind::Paragraph,
        text,
        bold: false,
        size: BODY_SIZE,
        line_spacing: BODY_LINE,
        space_before: 0.0,
        space_after: 6.0,
        lang,
    };

    let mut blocks = vec![heading(1, &doc.title)];
    if !doc.metadata.is_empty() {
        blocks.push(heading(2, INFO_HEADING));
        for (key, value) in &doc.metadata {
            blocks.push(paragraph(format!("{key}: {value}"), None));
        }
    }
    blocks.push(heading(2, BODY_HEADING));
    let content_lang = (doc.content_locale != doc.locale).then(|| doc.content_locale.clone());
    for text in &doc.paragraphs {
        blocks.push(paragraph(text.clone(), content_lang.clone()));
    }
    blocks
}

fn layout(blocks: &[Block], regular: &Metrics<'_>, bold: &Metrics<'_>) -> Vec<PageLayout> {
    let content_width = PAGE_WIDTH - 2.0 * MARGIN;
    let bottom = PAGE_HEIGHT - MARGIN - FOOTER_SPACE;
    let body_line_height = BODY_SIZE * BODY_LINE;

    let mut pages = vec![PageLayout::default()];
    let mut y = MARGIN;

    for (index, block) in blocks.iter().enumerate() {
        let metrics = if block.bold { bold } else { regular };
        let lines = wrap(&block.text, metrics, block.size, content_width);
        let line_height = block.size * block.line_spacing;

        if y > MARGIN {
            y += block.space_before;
        }
        // Un'intestazione non resta mai da sola in fondo alla pagina.
        let needed = match block.kind {
            BlockKind::Heading(_) => lines.len() as f32 * line_height + 2.0 * body_line_height,
            BlockKind::Paragraph => line_height,
        };
        if y + needed > bottom && y > MARGIN {
            pages.push(PageLayout::default());
            y = MARGIN;
        }

        for text in lines {
            if y + line_height > bottom {
                pages.push(PageLayout::default());
                y = MARGIN;
            }
            let baseline =
                y + (line_height - block.size) / 2.0 + block.size * metrics.ascent_ratio();
            if let Some(page) = pages.last_mut() {
                page.lines.push(Line {
                    block: index,
                    text,
                    x: MARGIN,
                    baseline,
                });
            }
            y += line_height;
        }
        y += block.space_after;
    }
    pages
}

/// Divide il testo in righe che non superano `max_width`.
///
/// Le righe interrotte terminano con uno spazio, così copiando il testo dal PDF
/// le parole non risultano attaccate.
fn wrap(text: &str, metrics: &Metrics<'_>, size: f32, max_width: f32) -> Vec<String> {
    let space = metrics.width(" ", size);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0f32;

    for word in text.split_whitespace() {
        let width = metrics.width(word, size);
        if !current.is_empty() && current_width + space + width <= max_width {
            current.push(' ');
            current.push_str(word);
            current_width += space + width;
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
            lines.push(std::mem::take(&mut current));
        }
        if width <= max_width {
            current.push_str(word);
            current_width = width;
        } else {
            // Parola più lunga della riga (es. un indirizzo web): si spezza per caratteri.
            let mut piece = String::new();
            let mut piece_width = 0.0f32;
            for c in word.chars() {
                let w = metrics.width(c.encode_utf8(&mut [0; 4]), size);
                if piece_width + w > max_width && !piece.is_empty() {
                    lines.push(std::mem::take(&mut piece));
                    piece_width = 0.0;
                }
                piece.push(c);
                piece_width += w;
            }
            current = piece;
            current_width = piece_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn write_pdf(
    doc: &Document,
    blocks: &[Block],
    pages: &[PageLayout],
    regular: &LoadedFont,
    bold: &LoadedFont,
    pdf_ua: bool,
) -> std::result::Result<Vec<u8>, KrillaError> {
    let mut configuration = ConfigurationBuilder::new();
    if pdf_ua {
        configuration = configuration.with_accessibility_validator(Accessibility::UA1);
    }
    let settings = SerializeSettings {
        configuration: configuration.finish().unwrap_or_default(),
        enable_tagging: true,
        ..Default::default()
    };

    let mut pdf = PdfDocument::new_with(settings);
    let mut identifiers: Vec<Vec<Identifier>> = blocks.iter().map(|_| Vec::new()).collect();
    let page_count = pages.len();
    let regular_metrics = regular.metrics().ok();

    for (page_index, page_layout) in pages.iter().enumerate() {
        let Some(page_settings) = PageSettings::from_wh(PAGE_WIDTH, PAGE_HEIGHT) else {
            continue;
        };
        let mut page = pdf.start_page_with(page_settings);
        let mut surface = page.surface();

        for line in &page_layout.lines {
            let block = &blocks[line.block];
            let font = if block.bold { &bold.font } else { &regular.font };
            let span = SpanTag::empty().with_lang(block.lang.as_deref());
            let id = surface.start_tagged(ContentTag::Span(span));
            surface.draw_text(
                Point::from_xy(line.x, line.baseline),
                font.clone(),
                block.size,
                &line.text,
                false,
                TextDirection::Auto,
            );
            surface.end_tagged();
            identifiers[line.block].push(id);
        }

        // Numero di pagina: artefatto, ignorato dagli screen reader durante la lettura.
        let label = format!("Pagina {} di {}", page_index + 1, page_count);
        let label_width = regular_metrics
            .as_ref()
            .map_or(0.0, |m| m.width(&label, FOOTER_SIZE));
        surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(
            ArtifactType::PageNumber,
        )));
        surface.draw_text(
            Point::from_xy(
                (PAGE_WIDTH - label_width) / 2.0,
                PAGE_HEIGHT - MARGIN / 2.0 - FOOTER_SIZE,
            ),
            regular.font.clone(),
            FOOTER_SIZE,
            &label,
            false,
            TextDirection::Auto,
        );
        surface.end_tagged();

        surface.finish();
        page.finish();
    }

    let mut tree = TagTree::new().with_lang(Some(doc.locale.clone()));
    for (block, ids) in blocks.iter().zip(identifiers) {
        if ids.is_empty() {
            continue;
        }
        let children: Vec<Node> = ids.into_iter().map(Node::from).collect();
        let group = match block.kind {
            BlockKind::Heading(level) => TagGroup::with_children(
                Tag::Hn(
                    NonZeroU16::new(level).unwrap_or(NonZeroU16::MIN),
                    Some(block.text.clone()),
                ),
                children,
            ),
            BlockKind::Paragraph => TagGroup::with_children(Tag::P, children),
        };
        tree.push(group);
    }
    pdf.set_tag_tree(tree);
    pdf.set_outline(build_outline(blocks, pages));

    let created = doc.created;
    let offset_minutes = created.offset().local_minus_utc() / 60;
    let date = DateTime::new(created.year().clamp(0, i32::from(u16::MAX)) as u16)
        .month(created.month() as u8)
        .day(created.day() as u8)
        .hour(created.hour() as u8)
        .minute(created.minute() as u8)
        .second(created.second() as u8)
        .utc_offset_hour((offset_minutes / 60) as i8)
        .utc_offset_minute((offset_minutes % 60).unsigned_abs() as u8);
    pdf.set_metadata(
        Metadata::new()
            .title(doc.title.clone())
            .language(doc.locale.clone())
            .creator(format!("{} {}", crate::APP_NAME, crate::APP_VERSION))
            .creation_date(date),
    );

    pdf.finish()
}

/// Segnalibri del documento (richiesti da PDF/UA): il titolo con le sezioni come figli.
fn build_outline(blocks: &[Block], pages: &[PageLayout]) -> Outline {
    let mut outline = Outline::new();
    let mut root: Option<OutlineNode> = None;

    for (index, block) in blocks.iter().enumerate() {
        let BlockKind::Heading(level) = block.kind else {
            continue;
        };
        let first_line = pages.iter().enumerate().find_map(|(page_index, page)| {
            page.lines
                .iter()
                .find(|line| line.block == index)
                .map(|line| (page_index, line))
        });
        let Some((page_index, line)) = first_line else {
            continue;
        };
        let destination = XyzDestination::new(
            page_index,
            Point::from_xy(line.x, (line.baseline - block.size).max(0.0)),
        );
        let node = OutlineNode::new(block.text.clone(), destination);
        if level == 1 {
            if let Some(previous) = root.take() {
                outline.push_child(previous);
            }
            root = Some(node.with_open(true));
        } else if let Some(root) = root.as_mut() {
            root.push_child(node);
        } else {
            outline.push_child(node);
        }
    }
    if let Some(root) = root {
        outline.push_child(root);
    }
    outline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_document;

    #[test]
    fn produces_tagged_pdf() {
        if find_system_fonts().is_err() {
            eprintln!("nessun carattere di sistema: test saltato");
            return;
        }
        let mut doc = sample_document();
        // Abbastanza testo da occupare più pagine.
        for i in 0..120 {
            doc.paragraphs
                .push(format!("Paragrafo numero {i}: la trascrizione continua con altro testo di prova per riempire la pagina."));
        }
        let bytes = render(&doc).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/StructTreeRoot"), "manca la struttura dei tag");
        assert!(text.contains("/Lang"), "manca la lingua");
        assert!(text.contains("pdfuaid:part"), "il PDF non dichiara la conformità PDF/UA");
    }

    #[test]
    fn passes_pdf_ua_validation() {
        let Ok(fonts) = find_system_fonts() else {
            return;
        };
        let doc = sample_document();
        let regular = LoadedFont::load(&fonts.regular).unwrap();
        let bold = LoadedFont::load(&fonts.bold).unwrap();
        let blocks = build_blocks(&doc);
        let pages = layout(&blocks, &regular.metrics().unwrap(), &bold.metrics().unwrap());
        match write_pdf(&doc, &blocks, &pages, &regular, &bold, true) {
            Ok(_) => {}
            Err(KrillaError::Validation(errors)) => panic!("errori PDF/UA: {errors:#?}"),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn wraps_long_text() {
        let Ok(fonts) = find_system_fonts() else {
            return;
        };
        let font = LoadedFont::load(&fonts.regular).unwrap();
        let metrics = font.metrics().unwrap();
        let text = "parola ".repeat(100);
        let lines = wrap(&text, &metrics, 12.0, 200.0);
        assert!(lines.len() > 5);
        for line in &lines {
            assert!(metrics.width(line.trim_end(), 12.0) <= 200.0);
        }
        let long = "x".repeat(300);
        assert!(wrap(&long, &metrics, 12.0, 100.0).len() > 3);
    }
}
