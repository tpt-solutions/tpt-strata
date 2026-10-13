"""tpt-strata: embeddable columnar SQL over CSV, Parquet and Python rows.

The engine is written in Rust. This package is a thin, typed surface over the
compiled module ``tpt_strata._native``.

Example::

    import tpt_strata as strata

    orders = strata.Table.from_csv("orders.csv")
    totals = strata.sql(
        {"orders": orders},
        "SELECT region, SUM(quantity) AS units FROM orders GROUP BY region",
    )
    print(totals)
    print(totals.to_rows())
"""

from ._native import (
    StrataError,
    Table,
    check,
    explain,
    sql,
    __version__,
)

__all__ = ["StrataError", "Table", "check", "explain", "sql", "__version__"]
