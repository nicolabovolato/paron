use serde::{Deserialize, Serialize};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::{IntoParams, ToSchema};

use crate::application::repository::{ListQuery, Page, Paginated, SearchText};
use serde_utoipa_validate::{newtype, validate};

pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 50;

newtype! {
    pub Limit: u32 -> u32 {
        schema: ObjectBuilder::new()
            .schema_type(Type::Integer)
            .minimum(Some(1))
            .maximum(Some(MAX_LIMIT)),
        default: DEFAULT_LIMIT,
        deserialize: |raw: u32| {
            (1..=MAX_LIMIT)
                .contains(&raw)
                .then_some(raw)
                .ok_or(format!("expected an integer between 1 and {MAX_LIMIT}"))
        },
    }
}

newtype! {
    pub Search: String -> SearchText {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(SearchText::MIN))
            .max_length(Some(SearchText::MAX)),
        deserialize: |raw: String| SearchText::parse(raw),
    }
}

newtype! {
    pub Offset: u32 {
        schema: ObjectBuilder::new().schema_type(Type::Integer).minimum(Some(0)),
        default: 0,
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    search: Option<Search>,
    #[validate(default)]
    limit: Limit,
    #[validate(default)]
    offset: Offset,
}

impl From<ListParamsParsed> for ListQuery {
    fn from(params: ListParamsParsed) -> Self {
        Self {
            search: params.search.filter(|search| !search.is_empty()),
            page: Page {
                limit: params.limit,
                offset: params.offset,
            },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
}

impl<T, E: Into<T>> From<Paginated<E>> for PageResponse<T> {
    fn from(found: Paginated<E>) -> Self {
        Self {
            items: found.items.into_iter().map(Into::into).collect(),
            total: found.total,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use serde_utoipa_validate::ValidationErrors;
    use utoipa::openapi::path::ParameterIn;

    use super::*;

    // A query string gives each value as a string.
    fn page_from_query(query: serde_json::Value) -> Result<Page, ValidationErrors> {
        serde_json::from_value::<ListParams>(query)
            .expect("an invalid value is an error of `validated`, not of deserialization")
            .validated()
            .map(|params| ListQuery::from(params).page)
    }

    #[test]
    fn validated_without_limit_and_offset_returns_the_defaults() {
        let page = page_from_query(json!({}));

        assert_eq!(
            page.unwrap(),
            Page {
                limit: DEFAULT_LIMIT,
                offset: 0
            }
        );
    }

    #[test]
    fn validated_with_limit_and_offset_returns_them() {
        let page = page_from_query(json!({ "limit": "7", "offset": "20" }));

        assert_eq!(
            page.unwrap(),
            Page {
                limit: 7,
                offset: 20
            }
        );
    }

    #[test]
    fn validated_with_limit_outside_the_range_returns_error_for_limit() {
        for limit in ["0", &(MAX_LIMIT + 1).to_string()] {
            let page = page_from_query(json!({ "limit": limit }));

            assert_eq!(
                page.unwrap_err()
                    .into_fields()
                    .into_keys()
                    .collect::<Vec<_>>(),
                ["limit"],
                "{limit}"
            );
        }
    }

    #[test]
    fn validated_with_unreadable_limit_and_offset_returns_error_for_both() {
        let page = page_from_query(json!({ "limit": "all", "offset": "some" }));

        assert_eq!(
            page.unwrap_err()
                .into_fields()
                .into_keys()
                .collect::<Vec<_>>(),
            ["limit", "offset"]
        );
    }

    #[test]
    fn into_params_limit_states_its_range_and_default() {
        let query_parameters =
            serde_json::to_value(ListParams::into_params(|| Some(ParameterIn::Query))).unwrap();

        let limit = query_parameters
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "limit")
            .unwrap();
        assert_eq!(limit["schema"]["minimum"], 1);
        assert_eq!(limit["schema"]["maximum"], MAX_LIMIT);
        assert_eq!(limit["schema"]["default"], DEFAULT_LIMIT);
    }

    #[test]
    fn into_params_search_bounds_match_the_parser() {
        let query_parameters =
            serde_json::to_value(ListParams::into_params(|| Some(ParameterIn::Query))).unwrap();

        let search = query_parameters
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "search")
            .unwrap();
        let bound =
            |keyword: &str| usize::try_from(search["schema"][keyword].as_u64().unwrap()).unwrap();
        let (min, max) = (bound("minLength"), bound("maxLength"));

        assert!(SearchText::parse("a".repeat(min)).is_ok());
        assert!(SearchText::parse("a".repeat(max)).is_ok());
        assert!(SearchText::parse("a".repeat(max + 1)).is_err());
    }
}
