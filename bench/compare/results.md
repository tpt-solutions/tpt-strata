# Comparison run
- rows (facts): 1000000, dims: 1000
- timed runs per query: 5 (after one warm-up), median reported
- logical CPUs: 20
- tpt-strata 1.0.1 (path dependency), DataFusion 55.2, DuckDB 1.10501

## Load (Parquet to engine format, median not used: single run)
| engine | load ms |
|---|---|
| tpt-strata (1 thread) | 104.2 |
| DataFusion (1 partition) | 37.5 |
| DataFusion (20 partitions) | 34.4 |
| DuckDB (1 thread) | 121.1 |
| DuckDB (20 threads) | 117.6 |

## Queries (median of 5 runs after one warm-up, ms)
| query | engine | median ms | result rows | matches tpt-strata |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | tpt-strata (1 thread) | 34.7 | 8 | yes |
| Q1 group by + 3 aggregates | DataFusion (1 partition) | 8.50 | 8 | yes |
| Q1 group by + 3 aggregates | DataFusion (all cores) | 4.22 | 8 | yes |
| Q1 group by + 3 aggregates | DuckDB (1 thread) | 23.4 | 8 | yes |
| Q1 group by + 3 aggregates | DuckDB (all cores) | 7.48 | 8 | yes |
| Q2 filter + sort + limit | tpt-strata (1 thread) | 53.7 | 100 | yes |
| Q2 filter + sort + limit | DataFusion (1 partition) | 4.20 | 100 | yes |
| Q2 filter + sort + limit | DataFusion (all cores) | 2.01 | 100 | yes |
| Q2 filter + sort + limit | DuckDB (1 thread) | 13.1 | 100 | yes |
| Q2 filter + sort + limit | DuckDB (all cores) | 3.80 | 100 | yes |
| Q3 join + group by | tpt-strata (1 thread) | 103.7 | 20 | yes |
| Q3 join + group by | DataFusion (1 partition) | 9.39 | 20 | yes |
| Q3 join + group by | DataFusion (all cores) | 4.41 | 20 | yes |
| Q3 join + group by | DuckDB (1 thread) | 6.62 | 20 | yes |
| Q3 join + group by | DuckDB (all cores) | 5.69 | 20 | yes |
