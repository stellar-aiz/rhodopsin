use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    InvalidInput,
    UnsupportedFeature,
    MissingFont,
    LayoutFailure,
    InternalError,
}

impl Category {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::InvalidInput => 2,
            Self::UnsupportedFeature => 3,
            Self::MissingFont => 4,
            Self::LayoutFailure => 5,
            Self::InternalError => 6,
        }
    }
}

/// 診断に入力由来の自由文を持たせない。セル番地は検証後にのみ生成する。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Location {
    pub sheet_index: usize,
    pub cell: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub status: &'static str,
    pub category: Category,
    pub reason: &'static str,
    pub location: Option<Location>,
}

impl Failure {
    pub fn new(category: Category, reason: &'static str) -> Self {
        Self {
            status: "failed",
            category,
            reason,
            location: None,
        }
    }

    pub fn at(mut self, sheet_index: usize, cell: Option<String>) -> Self {
        self.location = Some(Location { sheet_index, cell });
        self
    }

    pub fn input(reason: &'static str) -> Self {
        Self::new(Category::InvalidInput, reason)
    }

    pub fn unsupported(reason: &'static str) -> Self {
        Self::new(Category::UnsupportedFeature, reason)
    }
}

pub type Result<T> = std::result::Result<T, Failure>;
