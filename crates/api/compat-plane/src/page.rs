//! The pin's paginator, as the compat plane's lists answer it: `BasePaginator.paginate` (`apps/api/plane/utils/paginator.py:655`).
//!
//! A list that uses it reads the page size from `per_page`, the page from `cursor` (`size:page:previous`), refuses a size or a
//! cursor that is no number with a 400, and answers one envelope. S47 (work items) and the stickies stub (X07) are two of them; they
//! differ in the default page size (1000 and 20) and in what they list, not in how they page.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::PlaneError;

/// The most a page may hold: the paginator's `max_per_page`, which no list here raises.
const MAX_PER_PAGE: i64 = 1000;

pub(crate) fn invalid_detail(message: &str) -> PlaneError {
    PlaneError::BadRequest(json!({"detail": message}))
}

/// The page size: the request's `per_page`, or `default`; not more than 1000. Only a size that is no number, or is over the cap, is
/// refused here (a 400). A size of 0 or less is let through: the pin refuses it last, with its 500, in [`window`].
pub(crate) fn per_page(q: &HashMap<String, String>, default: i64) -> Result<i64, PlaneError> {
    let Some(raw) = q.get("per_page") else { return Ok(default) };
    let n: i64 = raw.trim().parse().map_err(|_| invalid_detail("Invalid per_page parameter."))?;
    if n > MAX_PER_PAGE.max(default) {
        return Err(invalid_detail("Invalid per_page value. Cannot exceed 1000."));
    }
    Ok(n)
}

/// What a cursor says: `size:page:previous`, all three numbers (the size may be a float).
pub(crate) struct Cursor {
    /// The page size the cursor was made with. Only a cursor to the previous page is held to it.
    size: f64,
    page: i64,
    previous: bool,
}

/// The cursor asked for. No cursor is the first page. A window that would start before the first row, which is a page before the
/// first (or a negative size on a later one), is a 400 here, as it is on the pin, before the rest of the query is read.
pub(crate) fn read_cursor(q: &HashMap<String, String>, limit: i64) -> Result<Cursor, PlaneError> {
    let Some(raw) = q.get("cursor") else { return Ok(Cursor { size: 0.0, page: 0, previous: false }) };
    let invalid = || invalid_detail("Invalid cursor parameter.");
    let parts: Vec<&str> = raw.split(':').collect();
    let [size, page, previous] = parts.as_slice() else { return Err(invalid()) };
    let size = match size.trim().parse::<i64>() {
        Ok(n) => n as f64,
        Err(_) => size.trim().parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(invalid)?,
    };
    let page: i64 = page.trim().parse().map_err(|_| invalid())?;
    let previous: i64 = previous.trim().parse().map_err(|_| invalid())?;
    if page.saturating_mul(limit) < 0 {
        return Err(invalid_detail("Error in parsing"));
    }
    Ok(Cursor { size, page, previous: previous != 0 })
}

/// The pin's last checks, which its paginator makes once everything else has been read, and which it answers with its catch-all 500
/// (recorded in S47, in this order, after each 400 above): a size of 0 or less, which it divides by or slices with (PQ-402), and a
/// cursor to the previous page whose size is not the page size, which it slices negatively (PQ-403). Gives the size and the page.
pub(crate) fn window(limit: i64, cursor: &Cursor) -> Result<(u32, u32), PlaneError> {
    if limit <= 0 {
        return Err(PlaneError::internal(
            "per_page <= 0: the pin's paginator divides by it or slices negatively (PQ-402)",
        ));
    }
    if cursor.previous && cursor.size != limit as f64 {
        return Err(PlaneError::internal(
            "a previous-page cursor whose size is not per_page: the pin slices negatively (PQ-403)",
        ));
    }
    let size = u32::try_from(limit).map_err(|_| PlaneError::internal("per_page does not fit a page size"))?;
    Ok((size, u32::try_from(cursor.page).unwrap_or(u32::MAX)))
}

/// The answer: the page's rows, and where the pages before and after it are. `size` and `page` are what [`window`] gave, `total` is
/// how many rows the list has in all, and `has_next` whether the page after this one has any.
pub(crate) fn envelope(total: u64, size: u32, page: u32, has_next: bool, results: Vec<Value>) -> Value {
    json!({
        "grouped_by": null,
        "sub_grouped_by": null,
        "total_count": total,
        "next_cursor": format!("{size}:{}:0", i64::from(page) + 1),
        "prev_cursor": format!("{size}:{}:1", i64::from(page) - 1),
        "next_page_results": has_next,
        "prev_page_results": page > 0,
        "count": results.len(),
        "total_pages": total.div_ceil(u64::from(size)),
        "total_results": total,
        "extra_stats": null,
        "results": results,
    })
}
