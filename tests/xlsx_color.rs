use rhodopsin::xlsx::{
    color::{ColorContext, tint},
    styles::{Color, ColorSource},
    theme::{Theme, ThemeFonts},
};

fn context() -> ColorContext<'static> {
    ColorContext {
        theme: None,
        indexed: &[],
        automatic: None,
        foreground: None,
        background: None,
    }
}

#[test]
fn 既定パレットとカスタムパレットを解決する() {
    for (index, expected) in [
        (0, [0, 0, 0]),
        (9, [255, 255, 255]),
        (10, [255, 0, 0]),
        (24, [153, 153, 255]),
        (63, [51, 51, 51]),
    ] {
        assert_eq!(
            Color {
                source: ColorSource::Indexed(index),
                tint: 0.0
            }
            .resolve(&context())
            .unwrap(),
            expected
        );
    }
    let palette = [[1, 2, 3], [4, 5, 6]];
    let ctx = ColorContext {
        indexed: &palette,
        ..context()
    };
    assert_eq!(
        Color {
            source: ColorSource::Indexed(1),
            tint: 0.0
        }
        .resolve(&ctx)
        .unwrap(),
        [4, 5, 6]
    );
    assert_eq!(
        Color {
            source: ColorSource::Indexed(2),
            tint: 0.0
        }
        .resolve(&ctx)
        .unwrap_err()
        .reason,
        "indexed_color_missing"
    );
}

#[test]
fn テーマ色に濃淡を適用する() {
    let mut theme = Theme {
        colors: [[0; 3]; 12],
        major: ThemeFonts::default(),
        minor: ThemeFonts::default(),
    };
    theme.colors[4] = [255, 0, 0];
    let ctx = ColorContext {
        theme: Some(&theme),
        ..context()
    };
    assert_eq!(
        Color {
            source: ColorSource::Theme(4),
            tint: 0.5
        }
        .resolve(&ctx)
        .unwrap(),
        [255, 128, 128]
    );
    assert_eq!(
        Color {
            source: ColorSource::Theme(4),
            tint: -0.5
        }
        .resolve(&ctx)
        .unwrap(),
        [128, 0, 0]
    );
    assert_eq!(
        Color {
            source: ColorSource::Theme(4),
            tint: 0.0
        }
        .resolve(&context())
        .unwrap_err()
        .reason,
        "theme_missing"
    );
}

#[test]
fn 自動色とシステム色には明示コンテキストが必要() {
    for source in [
        ColorSource::Automatic,
        ColorSource::Indexed(64),
        ColorSource::Indexed(65),
    ] {
        assert!(Color { source, tint: 0.0 }.resolve(&context()).is_err());
    }
    let ctx = ColorContext {
        automatic: Some([1, 2, 3]),
        foreground: Some([4, 5, 6]),
        background: Some([7, 8, 9]),
        ..context()
    };
    for (source, expected) in [
        (ColorSource::Automatic, [1, 2, 3]),
        (ColorSource::Indexed(64), [4, 5, 6]),
        (ColorSource::Indexed(65), [7, 8, 9]),
    ] {
        assert_eq!(Color { source, tint: 0.0 }.resolve(&ctx).unwrap(), expected);
    }
}

#[test]
fn 濃淡の端点と無変換を保持する() {
    for r in [0, 1, 63, 128, 254, 255] {
        for g in [0, 1, 63, 128, 254, 255] {
            for b in [0, 1, 63, 128, 254, 255] {
                let rgb = [r, g, b];
                assert_eq!(tint(rgb, 0.0).unwrap(), rgb);
                assert_eq!(tint(rgb, -1.0).unwrap(), [0; 3]);
                assert_eq!(tint(rgb, 1.0).unwrap(), [255; 3]);
            }
        }
    }
    assert_eq!(tint([0, 128, 0], 0.5).unwrap(), [64, 255, 64]);
    assert_eq!(tint([0, 0, 128], 0.5).unwrap(), [64, 64, 255]);
    assert_eq!(tint([128; 3], -0.5).unwrap(), [64; 3]);
}

#[test]
fn 不正な公開モデルもパニックせず拒否する() {
    for amount in [f64::NAN, f64::INFINITY, -1.1, 1.1] {
        assert!(tint([0; 3], amount).is_err());
    }
    assert!(
        Color {
            source: ColorSource::Indexed(255),
            tint: 0.0
        }
        .resolve(&context())
        .is_err()
    );
}
