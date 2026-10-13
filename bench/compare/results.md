# Comparison run
- rows (facts): 1000000, dims: 1000
- timed runs per query: 5 (after one warm-up), median reported
- logical CPUs: 20
- tpt-strata 1.0.1 (path dependency), DataFusion 55.2, DuckDB 1.10501

## Load (Parquet to engine format, median not used: single run)
| engine | load ms |
|---|---|
| tpt-strata (1 thread) | 101.1 |
| DataFusion (1 partition) | 36.5 |
| DataFusion (20 partitions) | 31.6 |
| DuckDB (1 thread) | 117.6 |
| DuckDB (20 threads) | 117.4 |

## Queries (median of 5 runs after one warm-up, ms)
| query | engine | median ms | result rows | matches tpt-strata |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | tpt-strata (1 thread) | 107.4 | 8 | yes |
| Q1 group by + 3 aggregates | DataFusion (1 partition) | 8.06 | 8 | yes |
| Q1 group by + 3 aggregates | DataFusion (all cores) | 3.83 | 8 | yes |
| Q1 group by + 3 aggregates | DuckDB (1 thread) | 22.4 | 8 | yes |
| Q1 group by + 3 aggregates | DuckDB (all cores) | 6.76 | 8 | yes |
| Q2 filter + sort + limit | tpt-strata (1 thread) | 68.5 | 100 | yes |
| Q2 filter + sort + limit | DataFusion (1 partition) | 4.20 | 100 | yes |
| Q2 filter + sort + limit | DataFusion (all cores) | 2.61 | 100 | yes |
| Q2 filter + sort + limit | DuckDB (1 thread) | 13.7 | 100 | yes |
| Q2 filter + sort + limit | DuckDB (all cores) | 3.30 | 100 | yes |
| Q3 join + group by | tpt-strata (1 thread) | 268.8 | 20 | yes |
| Q3 join + group by | DataFusion (1 partition) | 9.44 | 20 | yes |
| Q3 join + group by | DataFusion (all cores) | 4.27 | 20 | yes |
| Q3 join + group by | DuckDB (1 thread) | 5.87 | 20 | yes |
| Q3 join + group by | DuckDB (all cores) | 4.55 | 20 | yes |
