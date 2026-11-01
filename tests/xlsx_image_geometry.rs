use rhodopsin::xlsx::{
    drawing::{Anchor, Marker},
    grid::Grid,
    image_geometry::GridGeometry,
};
fn grid() -> Grid {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="15" defaultColWidth="10"/><cols><col min="2" max="2" hidden="1"/></cols><sheetData><row r="2" ht="20"/><row r="3" hidden="1"/></sheetData></worksheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    Grid::parse(doc.root_element()).unwrap()
}
fn marker(column: u32, row: u32) -> Marker {
    Marker {
        column,
        row,
        column_offset: 12700,
        row_offset: 25400,
    }
}
#[test]
fn セル寸法とemuから画像のシート座標を求める() {
    let geometry = GridGeometry::new(&grid(), 4, 5, 7.0, 9).unwrap();
    let rect = geometry
        .anchor(&Anchor::OneCell {
            from: marker(3, 4),
            width_emu: 127000,
            height_emu: 254000,
        })
        .unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (53.5, 37.0, 10.0, 20.0)
    );
    let rect = geometry
        .anchor(&Anchor::TwoCell {
            from: marker(1, 1),
            to: marker(4, 5),
            resize_with_cells: true,
        })
        .unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (1.0, 2.0, 105.0, 50.0)
    );
}
#[test]
fn アンカーの上限と不正な矩形を拒否する() {
    assert!(GridGeometry::new(&grid(), 4, 5, 7.0, 8).is_err());
    let geometry = GridGeometry::new(&grid(), 4, 5, 7.0, 9).unwrap();
    assert!(
        geometry
            .anchor(&Anchor::OneCell {
                from: marker(5, 1),
                width_emu: 12700,
                height_emu: 12700
            })
            .is_err()
    );
    assert!(
        geometry
            .anchor(&Anchor::TwoCell {
                from: marker(4, 5),
                to: marker(1, 1),
                resize_with_cells: true
            })
            .is_err()
    );
    assert!(
        geometry
            .anchor(&Anchor::OneCell {
                from: marker(1, 1),
                width_emu: u64::MAX,
                height_emu: 12700
            })
            .is_err()
    );
}

#[test]
fn 画像をページ別に切り取り倍率を保持する() {
    use rhodopsin::xlsx::{
        drawing::{Picture, Raster},
        layout::{AxisItem, Page, Paper},
        pdf::graphics::Paint,
        range::CellRange,
    };
    let geometry = GridGeometry::new(&grid(), 4, 5, 7.0, 9).unwrap();
    let mut picture = Picture {
        crop: Default::default(),
        anchor: Anchor::OneCell {
            from: Marker {
                column: 1,
                row: 1,
                column_offset: 0,
                row_offset: 0,
            },
            width_emu: 1333500,
            height_emu: 190500,
        },
        raster: std::sync::Arc::new(Raster {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        }),
        print: true,
        hidden: false,
    };
    // 元画像の中央半分を拡大する場合と、外側へ拡張して余白を残す場合も同じページ切断を使う。
    for (crop, shift, width, y, height) in [
        (
            rhodopsin::xlsx::drawing::SourceCrop::default(),
            0.0,
            52.5,
            20.0,
            7.5,
        ),
        (
            rhodopsin::xlsx::drawing::SourceCrop {
                left: 25000.0,
                right: 25000.0,
                top: 25000.0,
                bottom: 25000.0,
            },
            -26.25,
            105.0,
            16.25,
            15.0,
        ),
        (
            rhodopsin::xlsx::drawing::SourceCrop {
                left: 12500.5,
                right: 37499.5,
                top: 0.0,
                bottom: 0.0,
            },
            -13.125525,
            105.0,
            20.0,
            7.5,
        ),
        (
            rhodopsin::xlsx::drawing::SourceCrop {
                left: -50000.0,
                right: -50000.0,
                top: -50000.0,
                bottom: -50000.0,
            },
            13.125,
            26.25,
            21.875,
            3.75,
        ),
    ] {
        picture.crop = crop;
        for (column, expected_x) in [(1, 10.0), (3, -16.25)] {
            let page = Page {
                sheet_index: 1,
                area_index: 0,
                paper: Paper {
                    width: 200.0,
                    height: 200.0,
                },
                scale: 0.5,
                origin_x: 10.0,
                origin_y: 20.0,
                columns: vec![AxisItem {
                    index: column,
                    offset: 0.0,
                    size: 52.5,
                    repeated: false,
                }]
                .into(),
                rows: vec![AxisItem {
                    index: 1,
                    offset: 0.0,
                    size: 15.0,
                    repeated: false,
                }]
                .into(),
                body: CellRange::parse("A1:D1").unwrap(),
            };
            let paints = geometry.picture_on_page(&picture, &page).unwrap();
            assert_eq!(paints.len(), 1);
            let Paint::Image {
                rect,
                clip: Some(clip),
                ..
            } = &paints[0]
            else {
                panic!("画像命令がない")
            };
            for (actual, expected) in [rect.x, rect.y, rect.width, rect.height].into_iter().zip([
                expected_x + shift,
                y,
                width,
                height,
            ]) {
                assert!((actual - expected).abs() < 1e-10);
            }
            assert_eq!(
                (clip.x, clip.y, clip.width, clip.height),
                (10.0, 20.0, 26.25, 7.5)
            );
            let expected_matrix = [
                width,
                0.0,
                0.0,
                height,
                expected_x + shift,
                200.0 - y - height,
            ];
            let pdf_page = rhodopsin::xlsx::pdf::Page {
                width: 200.0,
                height: 200.0,
                items: paints
                    .into_iter()
                    .map(rhodopsin::xlsx::pdf::Item::Paint)
                    .collect(),
            };
            let bytes = rhodopsin::xlsx::pdf::render(
                &[pdf_page],
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )
            .unwrap();
            let doc = lopdf::Document::load_mem(&bytes).unwrap();
            let content = doc.get_page_content(doc.get_pages()[&1]).unwrap();
            let operations = lopdf::content::Content::decode(&content)
                .unwrap()
                .operations;
            let matrix = operations.iter().find(|op| op.operator == "cm").unwrap();
            for (value, expected) in matrix.operands.iter().zip(expected_matrix) {
                assert!((f64::from(value.as_float().unwrap()) - expected).abs() < 0.0001);
            }
            assert!(operations.iter().any(|op| op.operator == "W"));
            assert!(operations.iter().any(|op| op.operator == "Do"));
        }
    }
}

#[test]
fn 画像の見出し領域と本文領域を分離する() {
    use rhodopsin::xlsx::{
        drawing::{Picture, Raster},
        layout::{AxisItem, Page, Paper},
        range::CellRange,
    };
    let geometry = GridGeometry::new(&grid(), 4, 5, 7.0, 9).unwrap();
    let mut picture = Picture {
        crop: Default::default(),
        anchor: Anchor::OneCell {
            from: Marker {
                column: 1,
                row: 1,
                column_offset: 0,
                row_offset: 0,
            },
            width_emu: 2667000,
            height_emu: 190500,
        },
        raster: std::sync::Arc::new(Raster {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        }),
        print: true,
        hidden: false,
    };
    let mut page = Page {
        sheet_index: 1,
        area_index: 0,
        paper: Paper {
            width: 200.0,
            height: 200.0,
        },
        scale: 1.0,
        origin_x: 10.0,
        origin_y: 20.0,
        columns: vec![
            AxisItem {
                index: 1,
                offset: 0.0,
                size: 52.5,
                repeated: true,
            },
            AxisItem {
                index: 4,
                offset: 52.5,
                size: 52.5,
                repeated: false,
            },
        ]
        .into(),
        rows: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 15.0,
            repeated: false,
        }]
        .into(),
        body: CellRange::parse("D1").unwrap(),
    };
    assert_eq!(geometry.picture_on_page(&picture, &page).unwrap().len(), 2);
    picture.print = false;
    assert!(
        geometry
            .picture_on_page(&picture, &page)
            .unwrap()
            .is_empty()
    );
    picture.print = true;
    page.columns = vec![AxisItem {
        index: 1,
        offset: 0.0,
        size: 53.0,
        repeated: false,
    }]
    .into();
    assert!(geometry.picture_on_page(&picture, &page).is_err());
}
