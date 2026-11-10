//! A small synthetic Word paper for tests and manual runs: Word-style
//! headings without a space in their style name (`Heading1`), manual
//! section numbers, an equation in a two-column table with a typed (1),
//! a figure with its caption, a table with a caption above it, typed
//! citations [1], [2] and typed "Fig. 1" / "Table 1" / "Eq. (1)" /
//! "Section 2", a reference list, two tracked changes by `daidai` and a
//! comment. Nothing in it comes from a real paper.

use std::io::Write;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
</Relationships>"#;

const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
</Relationships>"#;

const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>Synthetic Study</dc:title><dc:creator>ellrin</dc:creator>
</cp:coreProperties>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style>
<w:style w:type="paragraph" w:styleId="Author"><w:name w:val="Author"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading1"/></w:style>
<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/></w:style>
</w:styles>"#;

const COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:comment w:id="0" w:author="daidai" w:date="2026-11-02T09:30:00Z" w:initials="D"><w:p><w:r><w:t>Please cite a second source here.</w:t></w:r></w:p></w:comment>
</w:comments>"#;

fn p(style: Option<&str>, runs: &str) -> String {
    let ppr = style.map(|s| format!("<w:pPr><w:pStyle w:val=\"{s}\"/></w:pPr>")).unwrap_or_default();
    format!("<w:p>{ppr}{runs}</w:p>")
}

fn r(t: &str) -> String {
    format!("<w:r><w:t xml:space=\"preserve\">{t}</w:t></w:r>")
}

fn document() -> String {
    let mut b = String::new();
    b += &p(Some("Title"), &r("Synthetic Study of Folded Widgets"));
    b += &p(Some("Author"), &r("Ellrin Example, Daidai Example"));
    b += &p(Some("Heading1"), &r("1. Introduction"));
    b += &p(None, &(r("Folded widgets are common [1]. Earlier work measured their strength [2, 3].")));
    // a tracked insertion and deletion in one paragraph, and a comment
    b += &p(
        None,
        &format!(
            "{}<w:ins w:id=\"1\" w:author=\"daidai\" w:date=\"2026-11-01T10:00:00Z\">{}</w:ins>{}<w:del w:id=\"2\" w:author=\"daidai\" w:date=\"2026-11-01T10:05:00Z\"><w:r><w:delText xml:space=\"preserve\">very </w:delText></w:r></w:del>{}<w:commentRangeStart w:id=\"0\"/>{}<w:commentRangeEnd w:id=\"0\"/><w:r><w:commentReference w:id=\"0\"/></w:r>{}",
            r("We study "),
            r("folded "),
            r("widgets under load. The method is "),
            r("simple. "),
            r("Section 2 describes it"),
            r(" and Fig. 1 shows the setup."),
        ),
    );
    // a whole inserted paragraph
    b += &format!(
        "<w:p><w:pPr><w:rPr><w:ins w:id=\"3\" w:author=\"daidai\" w:date=\"2026-11-01T11:00:00Z\"/></w:rPr></w:pPr><w:ins w:id=\"4\" w:author=\"daidai\" w:date=\"2026-11-01T11:00:00Z\">{}</w:ins></w:p>",
        r("This paragraph was added in review.")
    );
    b += &p(Some("Heading1"), &r("2. Method"));
    b += &p(None, &r("The load follows Eq. (1), and Table 1 lists the widgets."));
    // equation table: OMML in the left cell, typed (1) in the right one
    b += r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="8000"/><w:gridCol w:w="1000"/></w:tblGrid><w:tr><w:tc><w:p><m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMath><m:r><m:t>F</m:t></m:r><m:r><m:t>=</m:t></m:r><m:r><m:t>k</m:t></m:r><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara></w:p></w:tc><w:tc><w:p><w:r><w:t>(1)</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    b += &p(None, &r("where k is the stiffness and x is the fold depth."));
    // figure: an image paragraph, then its caption
    b += r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Picture 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="1" name="image1.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="rId3"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    b += &p(Some("Caption"), &r("Figure 1. The test rig."));
    b += &p(Some("Caption"), &r("Table 1. Widgets and their stiffness."));
    b += r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Widget</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Stiffness</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>12</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    b += &p(Some("Heading1"), &r("3. Results"));
    b += &p(None, &r("Widget A held the load [3]."));
    // a whole deleted paragraph
    b += &format!(
        "<w:p><w:pPr><w:rPr><w:del w:id=\"5\" w:author=\"daidai\" w:date=\"2026-11-01T12:00:00Z\"/></w:rPr></w:pPr><w:del w:id=\"6\" w:author=\"daidai\" w:date=\"2026-11-01T12:00:00Z\"><w:r><w:delText>This sentence was removed.</w:delText></w:r></w:del></w:p>"
    );
    b += &p(Some("Heading1"), &r("References"));
    b += &p(None, &r("[1] A. Writer, Folded widgets, Journal of Widgets 3 (2020) 1-10."));
    b += &p(None, &r("[2] B. Author, Widget strength revisited, Widget Letters 5 (2021) 11-20."));
    b += &p(None, &r("[3] C. Person, Loads on widgets, Proceedings of Widget Day (2022) 21-30."));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{b}<w:sectPr/></w:body></w:document>"#
    )
}

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The synthetic .docx as bytes.
pub fn docx() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let doc = document();
    let parts: [(&str, &[u8]); 8] = [
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("docProps/core.xml", CORE.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/styles.xml", STYLES.as_bytes()),
        ("word/comments.xml", COMMENTS.as_bytes()),
        ("word/media/image1.png", PNG),
    ];
    for (name, bytes) in parts {
        zip.start_file(name, opts).expect("zip");
        zip.write_all(bytes).expect("zip");
    }
    zip.finish().expect("zip").into_inner()
}
