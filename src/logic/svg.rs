//! SVG → the forms code wants it in: data URIs, CSS, HTML and JSX.

use base64::Engine as _;

use super::xml::format_xml;

const XMLNS: &str = "http://www.w3.org/2000/svg";

pub struct Svg {
    /// Facts about the drawing: size, view box, element count, byte sizes.
    pub rows: Vec<(&'static str, String)>,
    /// Each output and what it is called.
    pub outputs: Vec<(&'static str, String)>,
    /// The markup, cleaned up, with an `xmlns`: what a browser renders as an image.
    pub image: String,
}

/// One attribute as written: its name and its value still in its quotes.
fn attributes(tag: &str) -> Vec<(&str, Option<&str>)> {
    let mut out = Vec::new();
    // Skip `<name`.
    let mut rest = tag.trim_start_matches('<');
    rest = &rest[rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len())..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with('>') || rest.starts_with("/>") || rest.starts_with("?>") {
            return out;
        }
        let end = rest.find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/').unwrap_or(rest.len());
        if end == 0 {
            rest = &rest[1..];
            continue;
        }
        let name = &rest[..end];
        rest = rest[end..].trim_start();
        let Some(after) = rest.strip_prefix('=') else {
            out.push((name, None));
            continue;
        };
        let after = after.trim_start();
        let len = match after.chars().next() {
            Some(q @ ('"' | '\'')) => after[1..].find(q).map(|i| i + 2).unwrap_or(after.len()),
            _ => after.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(after.len()),
        };
        out.push((name, Some(&after[..len])));
        rest = &after[len..];
    }
}

fn unquoted(v: &str) -> &str {
    v.trim_matches(|c| c == '"' || c == '\'')
}

/// The markup without its XML declaration, doctype and comments, and where
/// the root element's start tag ends.
fn body(minified: &str) -> Result<(String, usize), String> {
    let mut out = String::with_capacity(minified.len());
    let mut rest = minified;
    let mut root_end = None;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!") && !rest.starts_with("<![CDATA[") {
            // A doctype may carry an internal subset in brackets.
            let close = rest.find('>').unwrap_or(rest.len() - 1);
            Some(match rest[..close].find('[') {
                Some(_) => rest.find("]>").map(|i| i + 2).unwrap_or(close + 1),
                None => close + 1,
            })
        } else {
            None
        };
        match end {
            Some(e) => rest = &rest[e..],
            // Character data is content, whatever it looks like.
            None if rest.starts_with("<![CDATA[") => {
                let close = rest.find("]]>").map(|i| i + 3).unwrap_or(rest.len());
                out.push_str(&rest[..close]);
                rest = &rest[close..];
            }
            None => {
                let close = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
                if root_end.is_none() {
                    let name = rest[1..].split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or("");
                    if name != "svg" && !name.ends_with(":svg") {
                        return Err(format!("The root element is <{name}>, not <svg>"));
                    }
                    root_end = Some(out.len() + close);
                }
                out.push_str(&rest[..close]);
                rest = &rest[close..];
            }
        }
    }
    out.push_str(rest);
    let root_end = root_end.ok_or("There is no <svg> element")?;
    Ok((out, root_end))
}

/// Percent-encode only what a URL or a CSS `url("…")` cannot hold as is: far
/// shorter than Base64, and still readable.
fn uri_escape(svg: &str) -> String {
    // Inside double-quoted HTML and CSS, single quotes are the safe ones.
    let swap = !svg.contains('\'');
    let mut out = String::with_capacity(svg.len() + 16);
    let mut gap = false;
    for c in svg.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        if gap {
            out.push_str("%20");
            gap = false;
        }
        match c {
            '"' if swap => out.push('\''),
            '"' | '%' | '#' | '<' | '>' | '{' | '}' | '|' | '\\' | '^' | '`' => out.push_str(&format!("%{:02X}", c as u32)),
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                out.extend(c.encode_utf8(&mut buf).bytes().map(|b| format!("%{b:02X}")));
            }
        }
    }
    out
}

fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '-' || c == ':' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `fill: red; stroke-width: 2` → `{{ fill: "red", strokeWidth: "2" }}`.
fn jsx_style(css: &str) -> String {
    let pairs: Vec<String> = css
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| {
            let k = k.trim();
            // Custom properties keep their dashes and need quoting.
            let key = if k.starts_with("--") { format!("\"{k}\"") } else { camel(k) };
            format!("{key}: {}", serde_json::to_string(v.trim()).unwrap_or_default())
        })
        .collect();
    format!("{{{{ {} }}}}", pairs.join(", "))
}

fn jsx_tag(tag: &str, root: bool) -> String {
    let name_end = tag.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(tag.len());
    let mut out = tag[..name_end].to_string();
    for (name, value) in attributes(tag) {
        let key = match name {
            "class" => "className".to_string(),
            "for" => "htmlFor".to_string(),
            n if n.starts_with("data-") || n.starts_with("aria-") => n.to_string(),
            n => camel(n),
        };
        match value {
            Some(v) if name == "style" => out.push_str(&format!(" style={}", jsx_style(unquoted(v)))),
            Some(v) => out.push_str(&format!(" {key}={v}")),
            None => out.push_str(&format!(" {key}")),
        }
    }
    if root {
        out.push_str(" {...props}");
    }
    out.push_str(if tag.ends_with("/>") { " />" } else { ">" });
    out
}

/// A React component: attributes renamed the way JSX wants them.
fn jsx(clean: &str) -> Result<String, String> {
    let pretty = format_xml(clean, Some(2))?;
    let mut out = String::with_capacity(pretty.len() + 64);
    let mut rest = pretty.as_str();
    let mut root = true;
    while let Some(at) = rest.find('<') {
        // Braces mean "expression" in JSX, so literal ones are spelled out.
        out.push_str(&rest[..at].replace('{', "&#123;").replace('}', "&#125;"));
        rest = &rest[at..];
        // Character data (a <style> block, say) becomes a template string.
        if let Some(data) = rest.strip_prefix("<![CDATA[") {
            let end = data.find("]]>").unwrap_or(data.len());
            out.push_str(&format!("{{`{}`}}", data[..end].replace('\\', "\\\\").replace('`', "\\`")));
            rest = &data[(end + 3).min(data.len())..];
            continue;
        }
        let close = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
        if rest.starts_with("</") {
            out.push_str(&rest[..close]);
        } else {
            out.push_str(&jsx_tag(&rest[..close], root));
            root = false;
        }
        rest = &rest[close..];
    }
    out.push_str(rest);
    let body: String = out.lines().map(|l| format!("    {l}\n")).collect();
    Ok(format!("export function SvgIcon(props) {{\n  return (\n{body}  );\n}}"))
}

pub fn convert(src: &str) -> Result<Svg, String> {
    if src.trim().is_empty() {
        return Err("Paste SVG markup".into());
    }
    let minified = format_xml(src.trim(), None)?;
    let (mut clean, root_end) = body(&minified)?;
    let root = clean[..root_end].to_string();
    let attrs = attributes(&root);
    let attr = |name: &str| attrs.iter().find(|a| a.0 == name).and_then(|a| a.1).map(unquoted);

    let mut rows: Vec<(&'static str, String)> = Vec::new();
    if let (Some(w), Some(h)) = (attr("width"), attr("height")) {
        rows.push(("Size", format!("{w} × {h}")));
    }
    if let Some(vb) = attr("viewBox") {
        rows.push(("View box", vb.to_string()));
    }
    let elements = clean.matches('<').count() - clean.matches("</").count();
    rows.push(("Elements", super::thousands(elements as u64)));
    rows.push(("Original", super::plural(src.len(), "byte")));
    rows.push(("Minified", super::plural(clean.len(), "byte")));

    let inline = clean.clone();
    // On its own (as an image) an SVG needs its namespace; inline in HTML it does not.
    if attr("xmlns").is_none() {
        let name_end = clean.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(clean.len());
        clean.insert_str(name_end, &format!(" xmlns=\"{XMLNS}\""));
    }
    let uri = format!("data:image/svg+xml,{}", uri_escape(&clean));
    let base64 = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(&clean));
    let outputs = vec![
        ("Data URI", uri.clone()),
        ("Data URI (Base64)", base64),
        ("CSS", format!("background-image: url(\"{uri}\");")),
        ("HTML image", format!("<img src=\"{uri}\" alt=\"\">")),
        ("Inline SVG", inline.clone()),
        ("React (JSX)", jsx(&inline)?),
    ];
    Ok(Svg { rows, outputs, image: clean })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- made by hand -->\n<svg width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#06c\" stroke-width=\"2\" class=\"icon\">\n  <circle cx=\"12\" cy=\"12\" r=\"9\" style=\"fill: red; stroke-opacity: 0.5\"/>\n  <path d=\"M8 12l3 3 5-6\" stroke-linecap=\"round\" data-part=\"tick\"/>\n</svg>\n";

    #[test]
    fn data_uris_and_markup() {
        let s = convert(ICON).unwrap();
        let get = |k: &str| s.outputs.iter().find(|o| o.0 == k).unwrap().1.as_str();
        assert_eq!(
            get("Inline SVG"),
            "<svg width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#06c\" stroke-width=\"2\" class=\"icon\"><circle cx=\"12\" cy=\"12\" r=\"9\" style=\"fill: red; stroke-opacity: 0.5\"/><path d=\"M8 12l3 3 5-6\" stroke-linecap=\"round\" data-part=\"tick\"/></svg>"
        );
        assert_eq!(
            get("Data URI"),
            "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='24'%20height='24'%20viewBox='0%200%2024%2024'%20fill='none'%20stroke='%2306c'%20stroke-width='2'%20class='icon'%3E%3Ccircle%20cx='12'%20cy='12'%20r='9'%20style='fill:%20red;%20stroke-opacity:%200.5'/%3E%3Cpath%20d='M8%2012l3%203%205-6'%20stroke-linecap='round'%20data-part='tick'/%3E%3C/svg%3E"
        );
        assert!(get("CSS").starts_with("background-image: url(\"data:image/svg+xml,%3Csvg") && get("CSS").ends_with("%3E\");"));
        assert!(get("HTML image").starts_with("<img src=\"data:image/svg+xml,") && !get("HTML image")[10..].trim_end_matches("\" alt=\"\">").contains('"'));
        let b64 = get("Data URI (Base64)").strip_prefix("data:image/svg+xml;base64,").unwrap();
        let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).unwrap()).unwrap();
        assert_eq!(decoded, s.image);
        assert!(s.image.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\""));
        assert_eq!(s.rows[..3], [("Size", "24 × 24".to_string()), ("View box", "0 0 24 24".to_string()), ("Elements", "3".to_string())]);
    }

    #[test]
    fn react_component() {
        let s = convert(ICON).unwrap();
        assert_eq!(
            s.outputs.last().unwrap().1,
            "export function SvgIcon(props) {
  return (
    <svg width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#06c\" strokeWidth=\"2\" className=\"icon\" {...props}>
      <circle cx=\"12\" cy=\"12\" r=\"9\" style={{ fill: \"red\", strokeOpacity: \"0.5\" }} />
      <path d=\"M8 12l3 3 5-6\" strokeLinecap=\"round\" data-part=\"tick\" />
    </svg>
  );
}"
        );
        let linked = convert("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><use xlink:href=\"#a\"/><text>{x}</text></svg>").unwrap();
        let jsx = &linked.outputs.last().unwrap().1;
        assert!(jsx.contains("xmlnsXlink=\"http://www.w3.org/1999/xlink\"") && jsx.contains("<use xlinkHref=\"#a\" />") && jsx.contains("<text>&#123;x&#125;</text>"), "{jsx}");
        // An xmlns that is already there is not added twice.
        assert_eq!(linked.image.matches("xmlns=").count(), 1);
    }

    #[test]
    fn rejects_what_is_not_svg() {
        assert_eq!(convert("<html><body/></html>").err().unwrap(), "The root element is <html>, not <svg>");
        assert!(convert("<svg><path></svg>").err().unwrap().contains("line 1"));
        assert!(convert("  ").is_err());
        // Quotes are only swapped when that cannot break an attribute.
        let s = convert("<svg font-family=\"'Open Sans'\">é</svg>").unwrap();
        assert!(s.outputs[0].1.ends_with("font-family=%22'Open%20Sans'%22%3E%C3%A9%3C/svg%3E"), "{}", s.outputs[0].1);
    }
}
