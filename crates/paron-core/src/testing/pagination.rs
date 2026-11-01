use crate::application::repository::{ListQuery, Page};

pub fn everything() -> ListQuery {
    ListQuery {
        search: None,
        page: Page {
            limit: u32::MAX,
            offset: 0,
        },
    }
}
