fn main() {
    let name = std::env::args().nth(1).expect("a file name");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx")
        .join(&name);
    let (d, _) = wp_docx::open(&path).unwrap();
    println!("--- {name}");
    println!(
        "styles: {}  numbering empty: {}",
        d.styles.len(),
        d.numbering.is_empty()
    );
    println!("sections: {}", d.sections().len());
    for (r, s) in d.sections() {
        println!(
            "  {:?} {:?} {}x{}",
            r, s.page.orientation, s.page.width.0, s.page.height.0
        );
    }
    println!("blocks: {}", d.body.len());
    for (i, b) in d.body.iter().enumerate() {
        match b {
            wp_model::Block::Paragraph(p) => println!(
                "  {i} P style={:?} num={:?} breaks={} {:?}",
                p.props
                    .style
                    .and_then(|s| d.styles.get(s))
                    .map(|s| s.id.to_string()),
                p.props.numbering,
                p.rendered_page_breaks(),
                p.text()
            ),
            wp_model::Block::Table(t) => println!(
                "  {i} TBL grid={:?} rows={} cols={}",
                t.grid,
                t.rows.len(),
                t.columns()
            ),
            other => println!("  {i} {other:?}"),
        }
    }
    println!("labels: {:?}", d.list_labels());
    println!("text: {:?}", d.text());
}
