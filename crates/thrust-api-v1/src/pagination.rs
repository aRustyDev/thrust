//! Plane's cursor pagination (`BasePaginator` and `OffsetPaginator`, `apps/api/plane/utils/paginator.py`), as every v1 list uses it.
//!
//! A request names `per_page` (default and most: 1000) and a `cursor`, `value:page:is_prev`, where `value` is the page size the
//! cursor was made with and `page` counts from 0 (Plane calls it the offset). The answer is the 12-key envelope: the page's rows, how
//! many there are in all, how many pages, and the cursors of the next and the previous page (`next_cursor` is always
//! `limit:page+1:0` and `prev_cursor` `limit:page-1:1`, even where there is no such page; `next_page_results` and
//! `prev_page_results` say whether there is).
//!
//! What the pin does with a request that is wrong, which is reproduced as it is:
//!
//! * a `per_page` that is no integer is a 400 "Invalid per_page parameter.", and one above 1000 a 400 "Invalid per_page value.
//!   Cannot exceed 1000.";
//! * a `cursor` that is not three colon-separated numbers is a 400 "Invalid cursor parameter.", and a negative page a 400
//!   "Error in parsing";
//! * a `per_page` of 0 or less is a 500 (a division by zero; a negative slice), the owner's decision (Q-0033.2) is to reproduce it
//!   as a handled 500, and so is a cursor that goes back (`is_prev`) with a page size other than the request's (the pin slices a
//!   queryset from the end, which Django refuses).

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::fields::py_int;
use crate::V1Error;

/// The most rows a page has, and how many it has when the request does not say.
pub const MAX_PER_PAGE: i64 = 1000;

fn bad_request(message: impl Into<String>) -> V1Error {
    V1Error::Detail(StatusCode::BAD_REQUEST, message.into())
}

/// What a request asks of a page, read and not yet checked against the pin's second round of rules ([`PageRequest::plan`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRequest {
    per_page: i64,
    /// The first number of the cursor: the page size it was made with.
    cursor_value: f64,
    page: i64,
    is_prev: bool,
}

/// `Cursor.from_string`: `value:page:is_prev`, where `value` is a float if it has a `.` and an integer if not.
fn parse_cursor(s: &str) -> Option<(f64, i64, bool)> {
    let mut bits = s.split(':');
    let (value, page, is_prev) = (bits.next()?, bits.next()?, bits.next()?);
    if bits.next().is_some() {
        return None;
    }
    let value = if value.contains('.') { value.trim().parse::<f64>().ok()? } else { py_int(value)? as f64 };
    Some((value, py_int(page)?, py_int(is_prev)? != 0))
}

impl PageRequest {
    /// Read `per_page` and `cursor` as the query gave them (`None` for a parameter that is not there).
    pub fn parse(per_page: Option<&str>, cursor: Option<&str>) -> Result<Self, V1Error> {
        let per_page = match per_page {
            None => MAX_PER_PAGE,
            Some(s) => py_int(s).ok_or_else(|| bad_request("Invalid per_page parameter."))?,
        };
        if per_page > MAX_PER_PAGE {
            return Err(bad_request(format!("Invalid per_page value. Cannot exceed {MAX_PER_PAGE}.")));
        }
        let default = format!("{per_page}:0:0");
        let (cursor_value, page, is_prev) =
            parse_cursor(cursor.unwrap_or(&default)).ok_or_else(|| bad_request("Invalid cursor parameter."))?;
        Ok(Self { per_page, cursor_value, page, is_prev })
    }

    /// The window of rows the request means, or the pin's refusal of it.
    pub fn plan(self) -> Result<Plan, V1Error> {
        let limit = self.per_page.min(MAX_PER_PAGE);
        let offset = self.page.saturating_mul(limit);
        if offset < 0 {
            return Err(bad_request("Error in parsing"));
        }
        if limit <= 0 {
            return Err(V1Error::internal(
                "per_page <= 0: the pin's paginator divides by it or slices negatively (PQ-402)",
            ));
        }
        if self.is_prev && self.cursor_value != limit as f64 {
            return Err(V1Error::internal(
                "a previous-page cursor whose size is not per_page: the pin slices negatively (PQ-403)",
            ));
        }
        let (Ok(limit_rows), Ok(offset_rows)) = (usize::try_from(limit), usize::try_from(offset)) else {
            return Err(V1Error::internal("the page window does not fit in a usize"));
        };
        Ok(Plan { limit: limit_rows, page: self.page, offset: offset_rows })
    }
}

/// Which rows of the ordered list a page is: `limit` of them from row `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub limit: usize,
    pub page: i64,
    pub offset: usize,
}

impl Plan {
    /// The page's rows out of the whole list, already in the order the request asked for.
    pub fn slice<T>(&self, all: Vec<T>) -> Vec<T> {
        all.into_iter().skip(self.offset).take(self.limit).collect()
    }

    /// The 12-key envelope around a page's `results`, given how many rows there are in all.
    pub fn envelope(&self, results: Vec<Value>, total: usize) -> Value {
        json!({
            "grouped_by": null,
            "sub_grouped_by": null,
            "total_count": total,
            "next_cursor": format!("{}:{}:0", self.limit, self.page.saturating_add(1)),
            "prev_cursor": format!("{}:{}:1", self.limit, self.page.saturating_sub(1)),
            "next_page_results": total > self.offset.saturating_add(self.limit),
            "prev_page_results": self.page > 0,
            "count": results.len(),
            "total_pages": total.div_ceil(self.limit),
            "total_results": total,
            "extra_stats": null,
            "results": results,
        })
    }
}
