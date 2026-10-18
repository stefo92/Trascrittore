//! Esportazione in HTML5 semantico e accessibile.

use super::escape_xml as esc;
use crate::document::{Document, BODY_HEADING, INFO_HEADING};

const STYLE: &str = r#"
  :root { color-scheme: light dark; }
  body {
    font-family: system-ui, "Segoe UI", Arial, sans-serif;
    font-size: 1.125rem;
    line-height: 1.6;
    max-width: 46rem;
    margin: 2rem auto;
    padding: 0 1rem;
    color: #1a1a1a;
    background: #ffffff;
  }
  h1 { font-size: 1.8rem; line-height: 1.25; }
  h2 { font-size: 1.35rem; margin-top: 2rem; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.25rem 1rem; }
  dt { font-weight: bold; }
  dd { margin: 0; }
  @media (prefers-color-scheme: dark) {
    body { color: #f2f2f2; background: #121212; }
  }
  @media print {
    body { max-width: none; margin: 0; font-size: 11pt; color: #000; background: #fff; }
  }
"#;

pub(crate) fn render(doc: &Document) -> String {
    let lang = primary_subtag(&doc.locale);
    let content_lang = primary_subtag(&doc.content_locale);

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n");
    html.push_str(&format!("<html lang=\"{}\">\n<head>\n", esc(lang)));
    html.push_str("<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str(&format!(
        "<meta name=\"generator\" content=\"{} {}\">\n",
        crate::APP_NAME,
        crate::APP_VERSION
    ));
    html.push_str(&format!("<title>{}</title>\n", esc(&doc.title)));
    html.push_str(&format!("<style>{STYLE}</style>\n</head>\n<body>\n<main>\n"));
    html.push_str(&format!("<h1>{}</h1>\n", esc(&doc.title)));

    if !doc.metadata.is_empty() {
        html.push_str("<section aria-labelledby=\"informazioni\">\n");
        html.push_str(&format!(
            "<h2 id=\"informazioni\">{}</h2>\n<dl>\n",
            esc(INFO_HEADING)
        ));
        for (key, value) in &doc.metadata {
            html.push_str(&format!("<dt>{}</dt><dd>{}</dd>\n", esc(key), esc(value)));
        }
        html.push_str("</dl>\n</section>\n");
    }

    html.push_str("<section aria-labelledby=\"testo\">\n");
    html.push_str(&format!("<h2 id=\"testo\">{}</h2>\n", esc(BODY_HEADING)));
    let lang_attr = if content_lang.eq_ignore_ascii_case(lang) {
        String::new()
    } else {
        format!(" lang=\"{}\"", esc(content_lang))
    };
    html.push_str(&format!("<div{lang_attr}>\n"));
    for paragraph in &doc.paragraphs {
        html.push_str(&format!("<p>{}</p>\n", esc(paragraph)));
    }
    html.push_str("</div>\n</section>\n</main>\n</body>\n</html>\n");
    html
}

/// `it-IT` → `it`: in HTML basta la lingua principale.
fn primary_subtag(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or(locale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_document;

    #[test]
    fn semantic_structure() {
        let html = render(&sample_document());
        assert!(html.contains("<html lang=\"it\">"));
        assert!(html.contains("<title>Trascrizione di intervista</title>"));
        assert!(html.contains("<h1>Trascrizione di intervista</h1>"));
        assert!(html.contains("<dt>File audio</dt><dd>intervista.mp3</dd>"));
        assert!(html.contains("&lt;tag&gt; &amp; &quot;virgolette&quot;"));
        assert!(html.contains("<main>"));
    }

    #[test]
    fn marks_foreign_language_content() {
        let mut doc = sample_document();
        doc.content_locale = "en-US".into();
        assert!(render(&doc).contains("<div lang=\"en\">"));
    }
}
