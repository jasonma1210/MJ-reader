//! 软删除可见性守卫（better-harness：共享组件复用 + fail-closed 默认）。
//!
//! 所有涉及 `books` 的查询必须经此模块生成过滤子句，避免「主路径修、旁路漏」。
//! 不要在各处手写 `deleted_at IS NULL`——用下面的函数；JOIN 与守卫被打包成不可分割产物。
//!
//! 设计要点（架构师裁定）：
//! 1. 共享组件复用：6 个模块共用这一组纯函数，可见性语义变更（如将来加 `archived_at`）
//!    只改一处，编译即可全量覆盖。
//! 2. fail-closed 默认：`visible_join_books` 把守卫嵌进 JOIN 生成，调用方忘记加守卫 =
//!    根本拿不到「未带守卫的 JOIN」可用——缺守卫在源头即不可能。
//! 3. 可审计：所有站点 grep `soft_delete::` 即得全量清单（配合 `scripts/check-soft-delete.sh`）。

/// 生成 `alias.deleted_at IS NULL` 守卫（用于无别名的 `FROM books` 场景）。
///
/// # 示例
///
/// `visible_where("books")` 生成：
///
/// ```text
/// books.deleted_at IS NULL
/// ```
///
/// 断言见 `db/soft_delete_tests.rs`（doctest 无法覆盖：rustdoc 的 doctest 是独立 crate，
/// `crate::` 指向 doctest 自身根且私有模块不可达，写成 ```` ``` ```` 代码块恒报 E0433）。
pub fn visible_where(alias: &str) -> String {
    format!("{}.deleted_at IS NULL", alias)
}

/// 生成用于 AND 连接的守卫片段（含前导 AND）。
///
/// # 示例
///
/// `visible_and("b")` 生成：
///
/// ```text
///  AND b.deleted_at IS NULL
/// ```
pub fn visible_and(alias: &str) -> String {
    format!(" AND {}.deleted_at IS NULL", alias)
}

/// 生成 INNER JOIN books 并附带软删守卫——JOIN 与守卫不可分割，
/// 防止「写了 JOIN 忘了 AND」。
///
/// # 示例
///
/// `visible_join_books("b", "rp.book_id")` 生成：
///
/// ```text
/// JOIN books b ON b.id = rp.book_id AND b.deleted_at IS NULL
/// ```
pub fn visible_join_books(alias: &str, fk: &str) -> String {
    format!(
        "JOIN books {} ON {}.id = {} AND {}.deleted_at IS NULL",
        alias, alias, fk, alias
    )
}
