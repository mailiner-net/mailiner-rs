//! Print a loaded message as a chrome-free document (headers + sanitized body).

/// Envelope fields shown above the body in the print document.
pub struct PrintHeaders<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub cc: Option<&'a str>,
    pub subject: &'a str,
    pub date: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintError {
    /// `window.open` returned `None` (and iframe fallback did not run).
    PopupBlocked,
    Failed,
}

/// Full HTML document: escaped headers + the already-sanitized body fragment.
pub fn build_print_document(headers: &PrintHeaders<'_>, body_html: &str) -> String {
    let inner = print_document_inner(headers, body_html);
    format!("<!DOCTYPE html><html>{inner}</html>")
}

fn print_document_inner(headers: &PrintHeaders<'_>, body_html: &str) -> String {
    let subject = headers.subject.trim();
    let title = if subject.is_empty() {
        "(no subject)".to_string()
    } else {
        escape_html(subject)
    };
    let from = escape_html(headers.from);
    let date = escape_html(headers.date);
    let mut rows = String::new();
    push_header_row(&mut rows, "From", &from);
    if !headers.to.trim().is_empty() {
        push_header_row(&mut rows, "To", &escape_html(headers.to));
    }
    if let Some(cc) = headers.cc.map(str::trim).filter(|s| !s.is_empty()) {
        push_header_row(&mut rows, "Cc", &escape_html(cc));
    }
    push_header_row(&mut rows, "Date", &date);

    format!(
        "<head>\
<meta charset=\"utf-8\">\
<title>{title}</title>\
<style>\
@page {{ margin: 1.6cm; }}\
html, body {{ margin: 0; }}\
body {{ font-family: system-ui, sans-serif; color: #111; }}\
.mlnr-print-headers {{\
  font-size: 13px; line-height: 1.45;\
  border-bottom: 1px solid #bbb;\
  padding: 0 0 12px; margin: 0 0 16px;\
}}\
.mlnr-print-headers h1 {{\
  font-size: 1.15em; font-weight: 650; margin: 0 0 8px;\
}}\
.mlnr-print-headers table {{ border-collapse: collapse; width: 100%; }}\
.mlnr-print-headers th {{\
  text-align: left; font-weight: 600;\
  padding: 2px 16px 2px 0; vertical-align: top;\
  white-space: nowrap; color: #444; width: 1%;\
}}\
.mlnr-print-headers td {{ padding: 2px 0; vertical-align: top; word-break: break-word; }}\
</style>\
</head>\
<body>\
<header class=\"mlnr-print-headers\">\
<h1>{title}</h1>\
<table>{rows}</table>\
</header>\
<div class=\"mlnr-print-body\">{body_html}</div>\
</body>"
    )
}

fn push_header_row(out: &mut String, label: &str, value: &str) {
    out.push_str("<tr><th>");
    out.push_str(label);
    out.push_str("</th><td>");
    out.push_str(value);
    out.push_str("</td></tr>");
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Open a print-only document and call `print()`. Does not print Mailiner chrome.
pub fn open_print_document(html: &str) -> Result<(), PrintError> {
    #[cfg(feature = "web")]
    {
        match open_print_window(html) {
            Ok(()) => Ok(()),
            Err(PrintError::PopupBlocked) => {
                open_print_iframe(html).map_err(|_| PrintError::PopupBlocked)
            }
            Err(err) => open_print_iframe(html).or(Err(err)),
        }
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = html;
        Err(PrintError::Failed)
    }
}

#[cfg(feature = "web")]
fn html_document_inner(html: &str) -> &str {
    let s = html.strip_prefix("<!DOCTYPE html>").unwrap_or(html).trim();
    s.strip_prefix("<html>")
        .and_then(|s| s.strip_suffix("</html>"))
        .unwrap_or(s)
}

#[cfg(feature = "web")]
fn open_print_window(html: &str) -> Result<(), PrintError> {
    let window = web_sys::window().ok_or(PrintError::Failed)?;
    let popup = window
        .open_with_url_and_target("about:blank", "_blank")
        .map_err(|_| PrintError::Failed)?
        .ok_or(PrintError::PopupBlocked)?;
    let doc = popup.document().ok_or(PrintError::Failed)?;
    let root = doc.document_element().ok_or(PrintError::Failed)?;
    root.set_inner_html(html_document_inner(html));
    let _ = popup.focus();
    popup.print().map_err(|_| PrintError::Failed)?;
    Ok(())
}

#[cfg(feature = "web")]
fn open_print_iframe(html: &str) -> Result<(), PrintError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{HtmlElement, HtmlIFrameElement};

    let window = web_sys::window().ok_or(PrintError::Failed)?;
    let document = window.document().ok_or(PrintError::Failed)?;
    let body = document.body().ok_or(PrintError::Failed)?;
    let iframe: HtmlIFrameElement = document
        .create_element("iframe")
        .map_err(|_| PrintError::Failed)?
        .dyn_into()
        .map_err(|_| PrintError::Failed)?;
    iframe
        .set_attribute("aria-hidden", "true")
        .map_err(|_| PrintError::Failed)?;
    iframe
        .set_attribute("title", "Print preview")
        .map_err(|_| PrintError::Failed)?;
    let style = iframe.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("right", "0");
    let _ = style.set_property("bottom", "0");
    let _ = style.set_property("width", "0");
    let _ = style.set_property("height", "0");
    let _ = style.set_property("border", "0");

    let host: HtmlElement = iframe.clone().unchecked_into();
    let iframe_for_load = iframe.clone();
    let on_load = Closure::once(move || {
        let Some(frame_win) = iframe_for_load.content_window() else {
            return;
        };
        let frame = iframe_for_load.clone();
        let after = Closure::once(move || {
            if let Some(parent) = frame.parent_node() {
                let _ = parent.remove_child(&frame);
            }
        });
        frame_win.set_onafterprint(Some(after.as_ref().unchecked_ref()));
        after.forget();
        let _ = frame_win.focus();
        let _ = frame_win.print();
    });
    host.set_onload(Some(on_load.as_ref().unchecked_ref()));
    on_load.forget();
    iframe.set_srcdoc(html);
    body.append_child(&iframe).map_err(|_| PrintError::Failed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers<'a>(
        from: &'a str,
        to: &'a str,
        cc: Option<&'a str>,
        subject: &'a str,
        date: &'a str,
    ) -> PrintHeaders<'a> {
        PrintHeaders {
            from,
            to,
            cc,
            subject,
            date,
        }
    }

    #[test]
    fn includes_headers_and_unsanitized_body_fragment() {
        let html = build_print_document(
            &headers(
                "Ada <ada@example.com>",
                "Bob <bob@example.com>",
                Some("Cc Name <cc@example.com>"),
                "Hello <world>",
                "01 Jan 2026, 12:00",
            ),
            "<p>Body &amp; <img src=\"data:image/png;base64,AAA\"></p>",
        );
        assert!(html.contains("<th>From</th><td>Ada &lt;ada@example.com&gt;</td>"));
        assert!(html.contains("<th>To</th><td>Bob &lt;bob@example.com&gt;</td>"));
        assert!(html.contains("<th>Cc</th><td>Cc Name &lt;cc@example.com&gt;</td>"));
        assert!(html.contains("<th>Date</th><td>01 Jan 2026, 12:00</td>"));
        assert!(html.contains("<title>Hello &lt;world&gt;</title>"));
        assert!(html.contains("<h1>Hello &lt;world&gt;</h1>"));
        assert!(html.contains("<p>Body &amp; <img src=\"data:image/png;base64,AAA\"></p>"));
        assert!(!html.contains("mailiner-message-content"));
    }

    #[test]
    fn omits_empty_to_and_cc() {
        let html = build_print_document(
            &headers("ada@example.com", "  ", None, "Hi", "d"),
            "<p>x</p>",
        );
        assert!(html.contains("<th>From</th>"));
        assert!(!html.contains("<th>To</th>"));
        assert!(!html.contains("<th>Cc</th>"));
        assert!(html.contains("<p>x</p>"));
    }

    #[test]
    fn empty_subject_uses_placeholder() {
        let html = build_print_document(
            &headers("a@b.c", "c@d.e", None, "   ", "d"),
            "<pre>plain</pre>",
        );
        assert!(html.contains("<title>(no subject)</title>"));
        assert!(html.contains("<h1>(no subject)</h1>"));
        assert!(html.contains("<pre>plain</pre>"));
    }
}
