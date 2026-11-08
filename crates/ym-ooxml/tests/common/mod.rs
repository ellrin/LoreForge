//! Synthetic deck generated in code (no real deck content is committed).

use ym_ooxml::Package;

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn rels(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, t, target) in items {
        let ty = if t.starts_with("http") {
            t.to_string()
        } else {
            format!("{R}/{t}")
        };
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#
        ));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

const GRP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;

fn ph_sp(id: u32, name: &str, ph: &str, xfrm: Option<(i64, i64, i64, i64)>, text: &str) -> String {
    let x = xfrm
        .map(|(x, y, cx, cy)| {
            format!(r#"<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#)
        })
        .unwrap_or_default();
    let body = if text.is_empty() {
        "<a:p><a:endParaRPr lang=\"en-US\"/></a:p>".to_string()
    } else {
        format!(r#"<a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>{text}</a:t></a:r></a:p>"#)
    };
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{x}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>"#
    )
}

pub fn synthetic_deck() -> Vec<u8> {
    let mut p = Package::default();
    p.set(
        "[Content_Types].xml",
        format!(
            "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
<Override PartName=\"/ppt/slides/slide2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
<Override PartName=\"/ppt/notesSlides/notesSlide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>"
        )
        .into_bytes(),
    );
    p.set(
        "_rels/.rels",
        rels(&[
            ("rId1", "officeDocument", "ppt/presentation.xml"),
            ("rId2", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail", "docProps/thumbnail.jpeg"),
            ("rId3", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml"),
            ("rId4", "extended-properties", "docProps/app.xml"),
        ]),
    );
    p.set("docProps/core.xml", format!("{DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:creator>ellrin</dc:creator><dc:title>Secret title</dc:title></cp:coreProperties>").into_bytes());
    p.set("docProps/app.xml", format!("{DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Slides>2</Slides><TitlesOfParts>Secret title</TitlesOfParts></Properties>").into_bytes());
    p.set("docProps/thumbnail.jpeg", vec![0xFF, 0xD8, 0xFF, 0xD9]);
    p.set(
        "ppt/presentation.xml",
        format!(
            r#"{DECL}<p:presentation {NS} saveSubsetFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="9144000" cy="5143500" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/><p:defaultTextStyle><a:defPPr><a:defRPr lang="en-US"/></a:defPPr><a:lvl1pPr marL="0"><a:defRPr sz="1800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle><p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"><p14:section name="S" id="{{00000000-0000-0000-0000-000000000001}}"><p14:sldIdLst><p14:sldId id="256"/></p14:sldIdLst></p14:section></p14:sectionLst></p:ext></p:extLst></p:presentation>"#
        )
        .into_bytes(),
    );
    p.set(
        "ppt/_rels/presentation.xml.rels",
        rels(&[
            ("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
            ("rId2", "slide", "slides/slide1.xml"),
            ("rId3", "slide", "slides/slide2.xml"),
            ("rId4", "theme", "theme/theme1.xml"),
        ]),
    );
    p.set(
        "ppt/theme/theme1.xml",
        format!(
            r#"{DECL}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Synthetic"><a:themeElements><a:clrScheme name="S"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="00CC99"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="S"><a:majorFont><a:latin typeface="Heading Sans"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hant" typeface="Heading TC"/></a:majorFont><a:minorFont><a:latin typeface="Body Sans"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hant" typeface="Body TC"/></a:minorFont></a:fontScheme><a:fmtScheme name="S"/></a:themeElements></a:theme>"#
        )
        .into_bytes(),
    );
    // master: title + body placeholders, logo picture, text styles
    p.set(
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            r#"{DECL}<p:sldMaster {NS}><p:cSld><p:spTree>{GRP}{}{}<p:pic><p:nvPicPr><p:cNvPr id="7" name="Logo"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr userDrawn="1"/></p:nvPicPr><p:blipFill><a:blip r:embed="rId3"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="100" y="4900000"/><a:ext cx="500000" cy="200000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/><p:sldLayoutId id="2147483650" r:id="rId2"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn="l"><a:defRPr sz="2400" b="1"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/><a:ea typeface="+mj-ea"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr marL="228600" indent="-228600"><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/><a:defRPr sz="1400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/></a:defRPr></a:lvl1pPr><a:lvl2pPr marL="685800"><a:defRPr sz="1200"/></a:lvl2pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz="1000"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#,
            ph_sp(2, "Title Placeholder 1", r#"<p:ph type="title"/>"#, Some((254000, 203200, 8623300, 673100)), ""),
            ph_sp(3, "Text Placeholder 2", r#"<p:ph type="body" idx="1"/>"#, Some((254000, 1000000, 8623300, 3500000)), ""),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
            ("rId2", "slideLayout", "../slideLayouts/slideLayout2.xml"),
            ("rId3", "image", "../media/image1.png"),
            ("rId4", "theme", "../theme/theme1.xml"),
        ]),
    );
    p.set(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(
            r#"{DECL}<p:sldLayout {NS} type="title" preserve="1"><p:cSld name="Title Slide"><p:spTree>{GRP}{}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#,
            ph_sp(2, "Title 1", r#"<p:ph type="ctrTitle"/>"#, Some((400000, 1600000, 8000000, 800000)), ""),
            ph_sp(3, "Date 2", r#"<p:ph type="dt" sz="half" idx="10"/>"#, Some((0, 4800000, 2000000, 300000)), ""),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
    );
    p.set(
        "ppt/slideLayouts/slideLayout2.xml",
        format!(
            r#"{DECL}<p:sldLayout {NS} type="obj" preserve="1"><p:cSld name="1_Title+Content"><p:spTree>{GRP}{}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#,
            ph_sp(2, "Title 1", r#"<p:ph type="title"/>"#, None, ""),
            ph_sp(3, "Content 2", r#"<p:ph idx="1"/>"#, Some((254000, 900000, 8623300, 3600000)), ""),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/slideLayouts/_rels/slideLayout2.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
    );
    p.set("ppt/media/image1.png", b"\x89PNG-logo".to_vec());
    p.set("ppt/media/image2.png", b"\x89PNG-figure".to_vec());
    p.set("ppt/media/image3.png", b"\x89PNG-eqn".to_vec());

    // slide 1: cover
    p.set(
        "ppt/slides/slide1.xml",
        format!(
            r#"{DECL}<p:sld {NS}><p:cSld><p:spTree>{GRP}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
            ph_sp(2, "Title 1", r#"<p:ph type="ctrTitle"/>"#, None, "A Synthetic Cover"),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/slides/_rels/slide1.xml.rels",
        rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]),
    );

    // slide 2: hidden, every object kind
    let textbox = r#"<p:sp><p:nvSpPr><p:cNvPr id="3" name="TextBox 2"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="300000" y="1000000"/><a:ext cx="4000000" cy="1200000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="square" rtlCol="0"><a:spAutoFit/></a:bodyPr><a:lstStyle/><a:p><a:pPr marL="342900" indent="-342900" algn="just"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc><a:spcBef><a:spcPts val="600"/></a:spcBef><a:buFont typeface="+mj-lt"/><a:buAutoNum type="arabicPeriod"/></a:pPr><a:r><a:rPr lang="en-US" sz="1400" dirty="0"><a:latin typeface="Calibri"/></a:rPr><a:t>First </a:t></a:r><a:r><a:rPr lang="en-US" sz="1400" b="1" dirty="0"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:latin typeface="Calibri"/></a:rPr><a:t>key &amp; result</a:t></a:r></a:p><a:p><a:pPr lvl="1"><a:buChar char="-"/></a:pPr><a:r><a:rPr lang="zh-TW" altLang="en-US" sz="1200"><a:latin typeface="Calibri"/><a:ea typeface="Body TC"/></a:rPr><a:t>第二層</a:t></a:r><a:br><a:rPr lang="en-US" sz="1200"/></a:br><a:fld id="{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}" type="slidenum"><a:rPr lang="en-US"/><a:t>2</a:t></a:fld><a:endParaRPr lang="en-US" sz="1200"/></a:p></p:txBody></p:sp>"#;
    let card = r#"<p:sp><p:nvSpPr><p:cNvPr id="4" name="Card 3"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot="5400000"><a:off x="5000000" y="1000000"/><a:ext cx="1500000" cy="900000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 10000"/></a:avLst></a:prstGeom><a:solidFill><a:srgbClr val="E2F0D9"><a:alpha val="35000"/></a:srgbClr></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:prstDash val="dash"/></a:ln><a:effectLst><a:outerShdw blurRad="40000" dist="20000" dir="5400000"><a:srgbClr val="000000"><a:alpha val="38000"/></a:srgbClr></a:outerShdw></a:effectLst></p:spPr><p:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></p:style><p:txBody><a:bodyPr rtlCol="0" anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="1600" b="1"/><a:t>Label</a:t></a:r></a:p></p:txBody></p:sp>"#;
    let pic = r#"<p:pic><p:nvPicPr><p:cNvPr id="5" name="Picture 4" descr="figure"><a:extLst><a:ext uri="{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{00000000-0000-0000-0000-000000000005}"/></a:ext></a:extLst></p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"><a:alphaModFix amt="25000"/><a:extLst><a:ext uri="{BEBA8EAE-BF5A-486C-A8C5-ECC9F3942E4B}"><a14:imgProps xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><a14:imgLayer r:embed="rId9"><a14:imgEffect><a14:backgroundRemoval t="0" b="100000" l="0" r="100000"/></a14:imgEffect></a14:imgLayer></a14:imgProps></a:ext></a:extLst></a:blip><a:srcRect l="10000" t="0" r="25000" b="5000"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="300000" y="2500000"/><a:ext cx="2000000" cy="1500000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:ln w="28575"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></p:spPr></p:pic>"#;
    let cxn = r#"<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="6" name="Straight Arrow Connector 5"/><p:cNvCxnSpPr><a:cxnSpLocks/><a:stCxn id="3" idx="3"/><a:endCxn id="4" idx="1"/></p:cNvCxnSpPr><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm flipV="1"><a:off x="4300000" y="1450000"/><a:ext cx="700000" cy="100000"/></a:xfrm><a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom><a:ln w="19050"><a:solidFill><a:srgbClr val="000000"><a:alpha val="20000"/></a:srgbClr></a:solidFill><a:prstDash val="sysDash"/><a:tailEnd type="triangle" w="med" len="med"/></a:ln></p:spPr></p:cxnSp>"#;
    let grp = r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="7" name="Group 6"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="6000000" y="2500000"/><a:ext cx="1000000" cy="1000000"/><a:chOff x="0" y="0"/><a:chExt cx="2000000" cy="2000000"/></a:xfrm></p:grpSpPr><p:grpSp><p:nvGrpSpPr><p:cNvPr id="8" name="Group 7"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="1000000" y="1000000"/><a:ext cx="1000000" cy="1000000"/><a:chOff x="0" y="0"/><a:chExt cx="1000000" cy="1000000"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="9" name="Oval 8"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="300000" cy="300000"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="F7D9F0"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>1</a:t></a:r></a:p></p:txBody></p:sp></p:grpSp><p:sp><p:nvSpPr><p:cNvPr id="10" name="Freeform 9"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="100" y="100"/></a:lnTo></a:path></a:pathLst></a:custGeom><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="35CBB2"/></a:gs><a:gs pos="100000"><a:srgbClr val="92D050"/></a:gs></a:gsLst><a:lin ang="0" scaled="1"/></a:gradFill></p:spPr></p:sp></p:grpSp>"#;
    let table = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="11" name="Table 10"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="300000" y="4100000"/><a:ext cx="3000000" cy="500000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{5940675A-B579-460E-94D1-54222C63F5DA}</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w="1500000"/><a:gridCol w="1500000"/></a:tblGrid><a:tr h="250000"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1100"/><a:t>lr</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1100"/><a:t>1e-4</a:t></a:r></a:p></a:txBody><a:tcPr><a:solidFill><a:srgbClr val="F2DFF7"/></a:solidFill></a:tcPr></a:tc></a:tr><a:tr h="250000"><a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="en-US"/></a:p></a:txBody><a:tcPr/></a:tc><a:tc hMerge="1"><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="en-US"/></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
    let math = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><mc:Choice Requires="a14"><p:sp><p:nvSpPr><p:cNvPr id="12" name="TextBox 11"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="4000000" y="4100000"/><a:ext cx="800000" cy="200000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" rtlCol="0"><a:spAutoFit/></a:bodyPr><a:lstStyle/><a:p><a14:m><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:sSub><m:e><m:r><a:rPr lang="en-US" i="1"><a:latin typeface="Cambria Math"/></a:rPr><m:t>𝑑</m:t></m:r></m:e><m:sub><m:r><a:rPr lang="en-US" i="1"><a:latin typeface="Cambria Math"/></a:rPr><m:t>𝑖</m:t></m:r></m:sub></m:sSub></m:oMath></a14:m><a:r><a:rPr lang="en-US" sz="1100"/><a:t> is a distance</a:t></a:r></a:p></p:txBody></p:sp></mc:Choice><mc:Fallback xmlns=""><p:sp><p:nvSpPr><p:cNvPr id="12" name="TextBox 11"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="4000000" y="4100000"/><a:ext cx="800000" cy="200000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:blipFill><a:blip r:embed="rId3"/><a:stretch><a:fillRect l="-1000" r="-1000"/></a:stretch></a:blipFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="zh-TW" altLang="en-US"><a:noFill/></a:rPr><a:t> </a:t></a:r></a:p></p:txBody></p:sp></mc:Fallback></mc:AlternateContent>"#;
    let chart = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="13" name="Chart 12"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="7000000" y="100000"/><a:ext cx="1000000" cy="800000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId8"/></a:graphicData></a:graphic></p:graphicFrame>"#;
    let ink = r#"<p:contentPart r:id="rId7" xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"/>"#;
    p.set(
        "ppt/slides/slide2.xml",
        format!(
            r#"{DECL}<p:sld {NS} show="0"><p:cSld><p:spTree>{GRP}{}{textbox}{card}{pic}{cxn}{grp}{table}{math}{chart}{ink}</p:spTree><p:extLst><p:ext uri="{{BB962C8B-B14F-4D97-AF65-F5344CB8AC3E}}"><p14:creationId xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" val="1"/></p:ext></p:extLst></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr><p:extLst><p:ext uri="{{X}}"><p14:x xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"/></p:ext></p:extLst></p:sld>"#,
            ph_sp(2, "Title 1", r#"<p:ph type="title"/>"#, None, "Every object kind"),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/slides/_rels/slide2.xml.rels",
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout2.xml"),
            ("rId2", "image", "../media/image2.png"),
            ("rId3", "image", "../media/image3.png"),
            ("rId4", "notesSlide", "../notesSlides/notesSlide1.xml"),
        ]),
    );
    p.set(
        "ppt/notesSlides/notesSlide1.xml",
        format!(
            r#"{DECL}<p:notes {NS}><p:cSld><p:spTree>{GRP}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#,
            ph_sp(2, "Notes Placeholder 1", r#"<p:ph type="body" idx="1"/>"#, None, "Speaker note"),
        )
        .into_bytes(),
    );
    p.set(
        "ppt/notesSlides/_rels/notesSlide1.xml.rels",
        rels(&[("rId1", "slide", "../slides/slide2.xml")]),
    );
    p.to_bytes().unwrap()
}
