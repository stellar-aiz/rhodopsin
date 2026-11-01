use rhodopsin::xlsx::{
    page_borders::{Edge, Side, coalesce},
    styles::LineStyle,
};
fn edge(cell: &str, side: Side, from: [f64; 2], to: [f64; 2]) -> Edge {
    Edge {
        sheet_index: 1,
        cell: cell.into(),
        side,
        from,
        to,
        style: LineStyle::Thin,
        color: [0; 3],
    }
}
#[test]
fn 同じ共有辺だけを一本にし隣接線分は連結しない() {
    let edges = vec![
        edge("A1", Side::Right, [10.0, 0.0], [10.0, 20.0]),
        edge("B1", Side::Left, [10.0, -0.0], [10.0, 20.0]),
        edge("A2", Side::Right, [10.0, 20.0], [10.0, 40.0]),
        edge("A1", Side::Top, [0.0, 0.0], [10.0, 0.0]),
    ];
    let result = coalesce(&edges, 4).unwrap();
    assert_eq!(result.len(), 3);
    let vertical: Vec<_> = result
        .iter()
        .filter(|e| matches!(e.side, Side::Left | Side::Right))
        .collect();
    assert_eq!(vertical.len(), 2);
    assert_eq!(vertical[0].cell, "A1");
    assert_eq!(vertical[0].to, vertical[1].from);
}
#[test]
fn 共有辺の線種と色が違えば優先順位を推測しない() {
    let a = edge("A1", Side::Bottom, [0.0, 20.0], [10.0, 20.0]);
    for change_color in [false, true] {
        let mut b = edge("A2", Side::Top, [0.0, 20.0], [10.0, 20.0]);
        if change_color {
            b.color = [255, 0, 0];
        } else {
            b.style = LineStyle::Double;
        }
        let error = coalesce(&[a.clone(), b], 2).unwrap_err();
        assert_eq!(error.reason, "border_conflict");
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("A2"));
    }
}
#[test]
fn 部分的な重なりは同色でも二重描画しない() {
    let a = edge("A1", Side::Bottom, [-10.0, 20.0], [10.0, 20.0]);
    for range in [[0.0, 20.0], [-5.0, 5.0]] {
        let b = edge("B1", Side::Top, [range[0], 20.0], [range[1], 20.0]);
        assert_eq!(
            coalesce(&[b, a.clone()], 2).unwrap_err().reason,
            "border_partial_overlap"
        );
    }
}
#[test]
fn 不正位置と幾何と上限を検査する() {
    let valid = edge("A1", Side::Right, [0.0, 0.0], [0.0, 20.0]);
    assert_eq!(
        coalesce(std::slice::from_ref(&valid), 0)
            .unwrap_err()
            .reason,
        "page_border_work_limit"
    );
    for (from, to) in [
        ([f64::NAN, 0.0], [0.0, 20.0]),
        ([0.0, 0.0], [1.0, 20.0]),
        ([0.0, 20.0], [0.0, 0.0]),
        ([0.0, 0.0], [0.0, 0.0]),
    ] {
        let mut value = valid.clone();
        value.from = from;
        value.to = to;
        assert_eq!(
            coalesce(&[value], 1).unwrap_err().reason,
            "invalid_border_geometry"
        );
    }
    let mut value = valid.clone();
    value.cell = "顧客の機密".into();
    let error = coalesce(&[value], 1).unwrap_err();
    assert_eq!(error.reason, "invalid_border_location");
    assert!(!serde_json::to_string(&error).unwrap().contains("顧客"));
    let mut other = valid.clone();
    other.sheet_index = 2;
    assert_eq!(
        coalesce(&[valid, other], 2).unwrap_err().reason,
        "page_sheet_mismatch"
    );
}

#[test]
fn 罫線を明示幅の実線破線二重線へ変換する() {
    use rhodopsin::xlsx::{
        page_borders::{Stroke, paint},
        pdf::graphics::Paint,
    };
    let mut double = edge("A1", Side::Right, [10.0, 0.0], [10.0, 20.0]);
    double.style = LineStyle::Double;
    let paints = paint(&[double.clone(), double], 2, 2, |_| {
        Ok(Stroke {
            width: 0.5,
            dash: vec![],
            separation: Some(1.5),
        })
    })
    .unwrap();
    assert_eq!(paints.len(), 2);
    for (paint, x) in paints.iter().zip([9.25, 10.75]) {
        match paint {
            Paint::Line {
                from,
                to,
                width,
                dash,
                ..
            } => {
                assert_eq!(*from, [x, 0.0]);
                assert_eq!(*to, [x, 20.0]);
                assert_eq!(*width, 0.5);
                assert!(dash.is_empty());
            }
            _ => panic!("線以外を生成した"),
        }
    }
    let mut dashed = edge("A1", Side::Top, [0.0, 0.0], [20.0, 0.0]);
    dashed.style = LineStyle::Dashed;
    let paints = paint(&[dashed], 1, 1, |_| {
        Ok(Stroke {
            width: 0.75,
            dash: vec![3.0, 1.0],
            separation: None,
        })
    })
    .unwrap();
    assert!(matches!(&paints[0], Paint::Line {dash, ..} if dash == &[3.0, 1.0]));
}

#[test]
fn 罫線の不正描画規則と展開後の総量を拒否する() {
    use rhodopsin::xlsx::page_borders::{Stroke, paint};
    let edge = edge("A1", Side::Top, [0.0, 0.0], [20.0, 0.0]);
    for stroke in [
        Stroke {
            width: f64::NAN,
            dash: vec![],
            separation: None,
        },
        Stroke {
            width: 0.0,
            dash: vec![],
            separation: None,
        },
        Stroke {
            width: 1.0,
            dash: vec![0.0],
            separation: None,
        },
        Stroke {
            width: 1.0,
            dash: vec![1.0; 33],
            separation: None,
        },
        Stroke {
            width: 1.0,
            dash: vec![],
            separation: Some(3.0),
        },
    ] {
        let error = paint(std::slice::from_ref(&edge), 1, 1, |_| Ok(stroke.clone()))
            .err()
            .unwrap();
        assert_eq!(error.reason, "invalid_border_stroke");
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
    }
    let mut double = edge;
    double.style = LineStyle::Double;
    assert_eq!(
        paint(&[double], 1, 1, |_| Ok(Stroke {
            width: 0.5,
            dash: vec![],
            separation: Some(1.5),
        }))
        .err()
        .unwrap()
        .reason,
        "page_scene_item_limit"
    );
}
