//! soft_delete 守卫纯字符串单测（`*_tests.rs` 命名，避开 unwrap 棘轮）。
//! 仅做字符串等式断言，无需任何 unwrap / 数据库，编译即可跑。

use crate::db::soft_delete::{visible_and, visible_join_books, visible_where};

#[test]
fn visible_where_renders_alias_guard() {
    assert_eq!(visible_where("books"), "books.deleted_at IS NULL");
}

#[test]
fn visible_and_renders_leading_and_guard() {
    assert_eq!(visible_and("b"), " AND b.deleted_at IS NULL");
}

#[test]
fn visible_join_books_bundles_join_and_guard() {
    assert_eq!(
        visible_join_books("b", "rp.book_id"),
        "JOIN books b ON b.id = rp.book_id AND b.deleted_at IS NULL"
    );
}
