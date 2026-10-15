use titler::Titler;

#[test]
fn test_titler_generation() {
    let mut model = Titler::from_hub("Cyronius/titler", None).expect("failed to load model");
    
    let title = model.generate("how do I center a div horizontally with flexbox").expect("generation failed");
    assert!(!title.is_empty(), "Generated title should not be empty");
    assert!(title.to_lowercase().contains("flexbox") || title.to_lowercase().contains("div"),
        "Generated title should relate to div/flexbox, got: {title}");

    let short_title = model.generate("hi").expect("generation failed");
    assert!(!short_title.is_empty(), "Short title should not be empty");
}
