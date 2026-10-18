//! Esportazione in DOCX (Office Open XML) scritto direttamente.
//!
//! Il documento usa gli stili predefiniti di Word «heading 1»/«heading 2» (Titolo 1/Titolo 2),
//! così gli screen reader possono navigare per intestazioni; la lingua è impostata sia
//! nello stile predefinito sia sui paragrafi del testo trascritto.

use std::io::{Cursor, Write};

use anyhow::Result;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::escape_xml as esc;
use crate::document::{Document, BODY_HEADING, INFO_HEADING};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

pub(crate) fn render(doc: &Document) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let parts: [(&str, String); 8] = [
        ("[Content_Types].xml", content_types()),
        ("_rels/.rels", root_rels()),
        ("docProps/core.xml", core_properties(doc)),
        ("docProps/app.xml", app_properties()),
        ("word/_rels/document.xml.rels", document_rels()),
        ("word/document.xml", document_xml(doc)),
        ("word/styles.xml", styles_xml(doc)),
        ("word/settings.xml", settings_xml(doc)),
    ];
    for (name, content) in parts {
        zip.start_file(name, options)?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

fn content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"#
        .to_string()
}

fn root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#
        .to_string()
}

fn document_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>
</Relationships>"#
        .to_string()
}

fn core_properties(doc: &Document) -> String {
    let created = doc
        .created
        .with_timezone(&chrono::Utc)
        .format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>{title}</dc:title>
<dc:creator>{app}</dc:creator>
<dc:language>{lang}</dc:language>
<dcterms:created xsi:type="dcterms:W3CDTF">{created}</dcterms:created>
<dcterms:modified xsi:type="dcterms:W3CDTF">{created}</dcterms:modified>
</cp:coreProperties>"#,
        title = esc(&doc.title),
        app = crate::APP_NAME,
        lang = esc(&doc.content_locale),
    )
}

fn app_properties() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
<Application>{} {}</Application>
</Properties>"#,
        crate::APP_NAME,
        crate::APP_VERSION
    )
}

fn settings_xml(doc: &Document) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="{W_NS}">
<w:defaultTabStop w:val="708"/>
<w:characterSpacingControl w:val="doNotCompress"/>
<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>
<w:themeFontLang w:val="{lang}"/>
</w:settings>"#,
        lang = esc(&doc.locale)
    )
}

fn styles_xml(doc: &Document) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="{W_NS}">
<w:docDefaults>
<w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="{lang}"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="300" w:lineRule="auto"/></w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="240"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val="40"/><w:szCs w:val="40"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:unhideWhenUsed/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="360" w:after="120"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val="30"/><w:szCs w:val="30"/></w:rPr></w:style>
</w:styles>"#,
        lang = esc(&doc.locale)
    )
}

fn document_xml(doc: &Document) -> String {
    let mut body = String::new();
    body.push_str(&heading(1, &doc.title));

    if !doc.metadata.is_empty() {
        body.push_str(&heading(2, INFO_HEADING));
        for (key, value) in &doc.metadata {
            body.push_str(&format!(
                r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}: </w:t></w:r><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                esc(key),
                esc(value)
            ));
        }
    }

    body.push_str(&heading(2, BODY_HEADING));
    let lang = esc(&doc.content_locale);
    for paragraph in &doc.paragraphs {
        body.push_str(&format!(
            r#"<w:p><w:r><w:rPr><w:lang w:val="{lang}"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            esc(paragraph)
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W_NS}">
<w:body>
{body}
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1134" w:bottom="1134" w:left="1134" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>
</w:body>
</w:document>"#
    )
}

fn heading(level: u8, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading{level}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        esc(text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_document;
    use std::io::Read;

    #[test]
    fn package_contains_required_parts() {
        let bytes = render(&sample_document()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
            "docProps/core.xml",
        ] {
            assert!(archive.by_name(name).is_ok(), "manca {name}");
        }
        let mut document = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut document)
            .unwrap();
        assert!(document.contains(r#"<w:pStyle w:val="Heading1"/>"#));
        assert!(document.contains("&lt;tag&gt; &amp; &quot;virgolette&quot;"));
        assert!(document.contains("perché città è così."));
    }
}
