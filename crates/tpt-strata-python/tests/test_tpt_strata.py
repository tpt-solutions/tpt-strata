"""Tests for the tpt-strata Python binding. Run after installing the wheel:

    maturin build --release -o dist
    pip install dist/tpt_strata-*.whl
    python -m unittest discover -s tests -v
"""

import datetime
import os
import tempfile
import unittest

import tpt_strata as strata

ORDERS = "order_id,customer_id,quantity,region\n1,1,4,north\n2,2,1,south\n3,1,,north\n"
CUSTOMERS = "customer_id,name\n1,ann\n2,ben\n"


def tables():
    return {
        "orders": strata.Table.from_csv_text(ORDERS),
        "customers": strata.Table.from_csv_text(CUSTOMERS),
    }


class TableTests(unittest.TestCase):
    def test_from_csv_text_infers_types_and_nulls(self):
        t = strata.Table.from_csv_text(ORDERS)
        self.assertEqual(t.columns, ["order_id", "customer_id", "quantity", "region"])
        self.assertEqual(t.row_count, 3)
        self.assertEqual(len(t), 3)
        self.assertEqual(t.to_rows()[2], (3, 1, None, "north"))

    def test_from_csv_file_round_trips_through_to_csv(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "orders.csv")
            with open(path, "w", encoding="utf-8") as f:
                f.write(ORDERS)
            t = strata.Table.from_csv(path)
            self.assertEqual(t.to_csv().splitlines()[0], "order_id,customer_id,quantity,region")

    def test_from_csv_missing_file_raises_file_not_found(self):
        with self.assertRaises(FileNotFoundError):
            strata.Table.from_csv("/no/such/file.csv")

    def test_dates_and_timestamps_round_trip_as_datetime_values(self):
        rows = [
            (datetime.date(2026, 2, 10), datetime.datetime(2026, 2, 10, 12, 30, 0, 500000)),
            (datetime.date(1969, 12, 31), datetime.datetime(1969, 12, 31, 23, 59, 59)),
        ]
        t = strata.Table.from_rows(["day", "at"], [list(r) for r in rows])
        self.assertEqual(t.to_rows(), rows)
        result = strata.sql({"t": t}, "SELECT day FROM t WHERE day >= DATE '2026-01-01'")
        self.assertEqual(result.to_rows(), [(datetime.date(2026, 2, 10),)])

    def test_datetime_with_time_zone_is_rejected(self):
        aware = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
        with self.assertRaises(ValueError):
            strata.Table.from_rows(["at"], [[aware]])

    def test_from_rows_accepts_python_values(self):
        t = strata.Table.from_rows(
            ["name", "score", "ok"],
            [["ann", 1.5, True], ["ben", None, False]],
        )
        self.assertEqual(t.to_rows(), [("ann", 1.5, True), ("ben", None, False)])

    def test_from_rows_rejects_unsupported_python_types(self):
        with self.assertRaises(TypeError):
            strata.Table.from_rows(["x"], [[[1, 2]]])

    def test_integer_beyond_i64_is_rejected_not_rounded(self):
        with self.assertRaises(ValueError):
            strata.Table.from_rows(["big"], [[2**70]])

    def test_bool_is_not_treated_as_an_integer(self):
        t = strata.Table.from_rows(["flag"], [[True], [False]])
        self.assertEqual(t.to_rows(), [(True,), (False,)])

    def test_parquet_round_trip(self):
        t = strata.Table.from_csv_text(ORDERS)
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "orders.parquet")
            t.to_parquet(path)
            back = strata.Table.from_parquet(path)
            self.assertEqual(back.to_rows(), t.to_rows())
            only = strata.Table.from_parquet(path, columns=["region"])
            self.assertEqual(only.columns, ["region"])

    def test_repr_is_a_grid(self):
        self.assertIn("| region", repr(strata.Table.from_csv_text(ORDERS)))


class QueryTests(unittest.TestCase):
    def test_sql_join_and_group(self):
        result = strata.sql(
            tables(),
            "SELECT customers.name, SUM(orders.quantity) AS units FROM orders "
            "JOIN customers ON orders.customer_id = customers.customer_id "
            "GROUP BY customers.name ORDER BY name",
        )
        self.assertEqual(result.columns, ["name", "units"])
        self.assertEqual(result.to_rows(), [("ann", 4), ("ben", 1)])

    def test_null_comparisons_follow_sql_rules(self):
        result = strata.sql(tables(), "SELECT order_id FROM orders WHERE quantity IS NULL")
        self.assertEqual(result.to_rows(), [(3,)])

    def test_error_carries_its_kind(self):
        with self.assertRaises(strata.StrataError) as ctx:
            strata.sql(tables(), "SELECT nope FROM orders")
        self.assertEqual(ctx.exception.kind, "missing-column")
        self.assertIn("nope", str(ctx.exception))

    def test_sql_error_kind(self):
        with self.assertRaises(strata.StrataError) as ctx:
            strata.sql(tables(), "SELECT FROM orders")
        self.assertEqual(ctx.exception.kind, "sql")

    def test_non_table_value_is_a_type_error(self):
        with self.assertRaises(TypeError):
            strata.sql({"orders": "not a table"}, "SELECT 1 FROM orders")

    def test_explain_and_check(self):
        plan = strata.explain(tables(), "SELECT order_id FROM orders WHERE quantity > 1")
        self.assertIn("Filter quantity > 1", plan)
        self.assertIsNone(strata.check(tables(), "SELECT order_id FROM orders"))
        with self.assertRaises(strata.StrataError):
            strata.check(tables(), "SELECT order_id FROM nowhere")

    def test_version_is_exported(self):
        self.assertEqual(strata.__version__, "0.1.0")


if __name__ == "__main__":
    unittest.main()
