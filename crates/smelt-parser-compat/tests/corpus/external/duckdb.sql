# Vendored from DuckDB v1.5.0 by scripts/extract-sql-corpus.py.
# One statement per line. Do not hand-edit; re-run the script to refresh.
# See ./README.md for license/attribution notices.

select max(l_orderkey, 3) from lineitem
SELECT equi_width_bins(0, 10, 5, true)
SELECT equi_width_bins(1, 6000000, 7, true)
select equi_width_bins(0.0, 6.347, 30, true) AS boundaries
select equi_width_bins(0.0, 3.974, 40, true) AS boundaries
SELECT s, SUM(a) FROM test GROUP BY s ORDER BY s
SELECT g, COUNT(*), COUNT(s), MIN(s), MAX(s), STRING_AGG(s, ' ') FROM strings WHERE s IS NULL OR s <> 'hello' GROUP BY g ORDER BY g
SELECT SUM(1), SUM(NULL), SUM(33.3)
SELECT ANY_VALUE(dt::TIMESTAMPTZ), ANY_VALUE(t::TIMETZ) FROM five_dates
SELECT ANY_VALUE(s ORDER BY 5-i), ANY_VALUE(l ORDER BY 5-i), ANY_VALUE(r ORDER BY 5-i) FROM five_complex
SELECT approx_quantile(i, 0.5) FROM repro
select arg_min_null(b,a), arg_max_null(b,a) from blobs
select bool_and(NULL)
SELECT COVAR_POP(NULL, NULL), COVAR_SAMP(NULL, NULL) FROM integers
select histogram(1)
SELECT kahan_sum(n)::BIGINT FROM doubles
select kurtosis(i) from (values (0), (0), (0), (0), (0), (0)) tbl(i)
SELECT LAST(b) FROM tbl WHERE a=1
select mad(x) from (values ('32767'::DECIMAL(5,0)), ('-32768'::DECIMAL(5,0))) tbl(x)
select mode(name) from names
SELECT FIRST(i ORDER BY i), FIRST(i ORDER BY i DESC) FROM integers
SELECT "dest", percentile_cont(0.5) WITHIN GROUP (ORDER BY "arr_delay") AS "median_delay" FROM "flights" GROUP BY "dest"
select percentile_disc([0.25, 0.5, 0.75]) within group(order by i desc) from generate_series(0,100) tbl(i)
SELECT quantile_disc(r, 0.1), quantile_disc(r, 0.5), quantile_disc(r, 0.9) from quantile
select regr_avgx(NULL,NULL)
select regr_r2(NULL,NULL)
select k, regr_r2(v, v2) from aggr group by k ORDER BY ALL
select k, regr_syy(v, v2) from aggr group by k ORDER BY ALL
select skewness(1)
select round(var_pop(val), 1) from stddev_test where val is not null
SELECT g, GROUP_CONCAT(x) FROM strings GROUP BY g ORDER BY g
SELECT STRING_AGG(g::VARCHAR, ',' ORDER BY CONCAT(x, y) ASC) FROM strings ORDER BY 1
SELECT DISTINCT ON (i) i, j FROM integers ORDER BY i, j NULLS FIRST
SELECT sum(distinct value), GROUPING(course, type), course, type, COUNT(*), sum(distinct value), FROM students GROUP BY CUBE(course, type) ORDER BY all
SELECT DISTINCT ON (1) i, j FROM integers ORDER BY i LIMIT 1
SELECT DISTINCT ON (2) j, (SELECT DISTINCT ON (i) i FROM integers ORDER BY 1 LIMIT 1) FROM integers ORDER BY 2
SELECT DISTINCT ON (integers.i) i, j FROM integers ORDER BY 1, 2
SELECT SUM(a), COUNT(*), AVG(a) FROM test
SELECT i, j, SUM(k), COUNT(*), COUNT(k) FROM integers GROUP BY i, j ORDER BY 1, 2
SELECT LENGTH(NULL) FROM t0 GROUP BY NULL
SELECT UPPER(NULL) FROM t0 GROUP BY NULL
SELECT GROUPING(course), course, COUNT(*) FROM students GROUP BY course ORDER BY 1, 2, 3
SELECT GROUPING(course), GROUPING(type), course, type, COUNT(*) FROM students GROUP BY CUBE(course, type) HAVING GROUPING(course)=0 ORDER BY 1, 2, 3, 4, 5
SELECT course, type, COUNT(*) FROM students GROUP BY CUBE(course, type) ORDER BY GROUPING(course), GROUPING(type), 1, 2, 3
select course, type, count(*) from students group by (course, type) order by 1, 2, 3
select course as crs, type as tp, count(*) from students group by grouping sets (rollup (crs)), (), tp order by 1, 2, 3
SELECT 1 AS one FROM ( values (1,2), (3,2) ) t(a, b) HAVING false
SELECT * FROM exam WINDOW w AS (ORDER BY mark) QUALIFY row_number() OVER w = 1
SELECT quantile_cont(r::${type}, [0.15, 0.5, 0.9]) FROM quantiles
SELECT table_name, database_name, temporary FROM duckdb_tables() WHERE table_name='temp_tbl'
SELECT * FROM blablabla
select * from db2.person
SELECT db1.main.two_x_plus_y(x, y) FROM db1.tbl
SELECT * EXCLUDE (db1.s1.t.c) FROM db1.s1.t, db2.s1.t
SELECT db1.s1.t, db2.s1.t FROM db1.s1.t, db2.s1.t
SELECT database.schema.table.col FROM database.schema.table
SELECT schema.table FROM database.schema.table
SELECT COUNT(*)>0 FROM pragma_storage_info('db1.str_tbl') WHERE compression='ZSTD'
SELECT nextval('db1.seq')
SELECT tags['storage_version'] FROM duckdb_databases() WHERE database_name='default_version'
SELECT * FROM ${prefix}.t2
SELECT new_name.my_schema.one()
SELECT a % 2 AS x, SUM(a) AS s FROM (VALUES (1),(2),(3),(4)) t(a) GROUP BY alias.x HAVING alias.s >= 6 ORDER BY x
select a as "user" from test group by "user" order by "user"
SELECT t.x, t.y FROM (SELECT 42 x) t, (SELECT 84 y) t
SELECT s1.t.c, t.c, c FROM s1.t
SELECT i % 2 AS p, SUM(i) AS sum_i FROM integers GROUP BY p ORDER BY 1
SELECT "HeLlO" FROM test
SELECT alias(x) FROM (SELECT HeLlO as x FROM test) tbl
SELECT i AS j, COUNT(*) AS i FROM integers GROUP BY j HAVING j=1 ORDER BY i
SELECT true='0'
select cast(0.5::${src} as ${dst}) as x
SELECT UNNEST('[NULL, NULL , ]'::varchar[])
SELECT CAST('[ hello , world , ! ]' AS VARCHAR[])
SELECT col1::INT[] FROM null_tbl
SELECT * FROM tbl WHERE cast(col1 as int[]) = [1, 2, 2]
SELECT col::STRUCT(a INT, b VARCHAR)[] FROM struct_tbl2
SELECT $$[hello\ world, world]$$::VARCHAR[]
SELECT $$["\""]$$::VARCHAR[]; -- List with only a quote
select $$[NULL, 'null', 'nUlL', NuLl, "NULLz", "NULL"]$$::VARCHAR[] a, a::VARCHAR::VARCHAR[] b, a == b
SELECT $${key{with}bracket = value}$$::MAP(VARCHAR, VARCHAR)
SELECT $${=}$$::MAP(VARCHAR, VARCHAR)
SELECT ($${a: "can't", b: "you're", c: "i'm"}$$::STRUCT(a VARCHAR, b VARCHAR, c VARCHAR))
SELECT '{key_A:0}'::STRUCT(key_A INT, key_B VARCHAR)
SELECT $${first name: John, age: 30}$$::STRUCT("first name" VARCHAR, age INT)
SELECT $${{\"name\"}: John, age: 3}$$::STRUCT("{""name""}" VARCHAR, age INT)
SELECT $${description: "Special characters: \\, \", \', (, )"}$$::STRUCT(description VARCHAR)
SELECT $${@: "value", age: 30}$$::STRUCT("@" VARCHAR, age INT)
SELECT {"CamelCase": 1, "lowercase": 2, "UPPERCASE": 3}::MAP(VARCHAR, INT)
SELECT col::MAP(VARCHAR, MAP(VARCHAR, INT)) FROM VALUES ({ nested: MAP { 'inner_key': 707 } }) AS tab(col)
SELECT 15::TINYINT::BIT
SELECT 15::UHUGEINT::BIT
SELECT (32767)::SMALLINT::BIT
SELECT (170141183460469231731687303715884105727)::HUGEINT::BIT
SELECT CAST(1=0 AS VARCHAR)
SELECT CAST('t' AS BOOLEAN)
SELECT CAST('false' AS BOOLEAN)
WITH binary_string as (select replace('${binary}', '_', '') as str) SELECT list_sum([ (CASE WHEN str[i+1] = '0' THEN 0 ELSE 1 END) * (2 ** (len(str)-(i+1))) for i in range(len(str))])::INT == '${prefix}${binary}'::INT FROM binary_string
SELECT 1::BIGINT::VARCHAR, 1244295295289253::BIGINT::VARCHAR, (-2000000111551166)::BIGINT::VARCHAR
SELECT 2::DOUBLE::VARCHAR, 0.5::DOUBLE::VARCHAR, (-128.5)::DOUBLE::VARCHAR
SELECT '0xFF'::UINT8, '0xFFFF'::UINT16, '0xFFFFFFFF'::UINT32, '0xFFFFFFFFFFFFFFFF'::UINT64
SELECT * FROM (a NATURAL FULL OUTER JOIN b NATURAL FULL OUTER JOIN c) NATURAL FULL OUTER JOIN (d NATURAL FULL OUTER JOIN e)
select comment from duckdb_columns() where column_name='test_table_column_renamed'
SELECT f.x FROM query_table('tbl_int') as f(x)
SELECT nested_cte(2, '2,2,2,2')
SELECT parameterized_cte(42)
select test(4, 2)
select parameter_types[1] from duckdb_functions() where function_name = 'm' and function_type = 'macro' order by all
SELECT test('seq1', 'seq2', i) FROM integers
SELECT * FROM xtm('m.*')
SELECT * FROM xt_reg('^m')
SELECT * FROM car_pool_rollup(model, yyyy, hcnt:=4)
select my_agg(range) OVER () from range(2)
SELECT i0, j, i1 FROM integers
select trim(sql, chr(10)) from duckdb_views() where internal = false
SELECT 'OX' COLLATE NOACCENT ILIKE 'ö%'
SELECT 'öX' COLLATE NOACCENT NOT ILIKE 'Ö%'
select concat(a collate de, a) from tbl order by all
SELECT collate_test.s FROM collate_test ORDER BY 1 COLLATE NOCASE
SELECT * FROM employee WHERE managerid = 2
SELECT count(*) FROM read_csv('{DATA_DIR}/csv/drug_exposure.csv')
SELECT typeof(TestInteger), typeof(TestDouble), typeof(TestDate), typeof(TestText) FROM test LIMIT 1
SELECT i FROM test ORDER BY i
select count(*) from read_csv('{DATA_DIR}/csv/empty.csv', columns=STRUCT_PACK(d := 'BIGINT'), header=0, auto_detect = false)
select count(*) from read_csv_auto('{DATA_DIR}/csv/big_not_bool.csv', header = 0)
SELECT rsID, chr, pos, refb, altb FROM t1
select id, value, CAST(part AS INT) as part_cast, CAST(date AS DATE) as date_cast from read_csv_auto('{DATA_DIR}/csv/hive-partitioning/types/*/*/test.csv', HIVE_PARTITIONING=1) where (date_cast=CAST('2012-01-01' as DATE) AND concat(date_cast::VARCHAR, part_cast::VARCHAR) == '2012-01-011000') OR (part_cast=1337) ORDER BY date_cast
select filename.replace('\', '/').split('/')[-2] from read_csv_auto('{DATA_DIR}/csv/hive-partitioning/simple/*/*/test.csv', HIVE_PARTITIONING=1, FILENAME=1) order by 1
select * exclude (filename) from read_csv_auto('{DATA_DIR}/csv/hive-partitioning/mismatching_types/*/*.csv', HIVE_PARTITIONING=0, FILENAME=1, UNION_BY_NAME=1) order by 1
select * from '{DATA_DIR}/csv/nullbyte.csv'
SELECT COUNT(*) FROM v1
SELECT SUM(i) FROM s1.tbl
select count(*) from glob('~/rewoiarwiouw3rajkawrasdf790273489*.py') limit 10
SELECT COUNT(*) FROM glob('*/*.csv')
SELECT sum(a), sum(b), sum(c) FROM read_csv('{DATA_DIR}/csv/test/multi_column_integer.csv', COLUMNS=STRUCT_PACK(a := 'INTEGER', b := 'INTEGER', c := 'INTEGER'), auto_detect='true', delim = '|', buffer_size=30)
SELECT sum(a) FROM read_csv('{DATA_DIR}/csv/test/multi_column_integer_rn.csv', COLUMNS=STRUCT_PACK(a := 'INTEGER', b := 'INTEGER', c := 'INTEGER'), auto_detect='true', delim = '|', buffer_size=30)
SELECT COUNT(*) > 0 FROM read_csv('__TEST_DIR__/test.txt', columns={'c': 'VARCHAR'}, delim=NULL, header=0, quote=NULL, escape=NULL, auto_detect = false) WHERE contains(c, 'Optimizer')
SELECT typeof(phone) FROM phone_numbers LIMIT 1
SELECT * FROM read_csv( '{DATA_DIR}/csv/rejects/incorrect_columns/few_columns.csv', columns = {'a': 'INTEGER', 'b': 'INTEGER', 'c': 'INTEGER', 'd': 'INTEGER'}, store_rejects=true, auto_detect=false, header = 1)
SELECT SUM(num) FROM read_csv( '{DATA_DIR}/csv/error/mismatch/big_bad.csv', columns = {'num': 'INTEGER', 'str': 'VARCHAR'}, store_rejects = true, auto_detect=false)
SELECT COUNT(*) FROM ${tbl}
SELECT column1, column2, column3, parse_filename(filename) FROM read_csv('{DATA_DIR}/csv/filename_filter/*.csv', filename=true)
SELECT COUNT(*) FROM nfcstrings WHERE s COLLATE NFC = 'ü'
SELECT quote, escape FROM sniff_csv('__TEST_DIR__/out_2.csv')
SELECT string_split_regex(a, '[\r\n]+') FROM test ORDER BY a
SELECT quote, escape from sniff_csv('{DATA_DIR}/csv/16857.csv', ignore_errors = true)
SELECT count(*) FROM glob('__TEST_DIR__/file_size_bytes_parquet/*.parquet')
select row_group_id, bloom_filter_offset IS NOT NULL, bloom_filter_length IS NOT NULL from parquet_metadata('__TEST_DIR__/bloom5.parquet') order by row_group_id
SELECT struct_val.i FROM bigint_file_first WHERE struct_val.i='042' ORDER BY ALL
SELECT struct_val.f, struct_val.i FROM integer_file_first WHERE struct_val.i IS NULL
select count(*) from parquet_scan('{DATA_DIR}/parquet-testing/glob/*')
select count(*) from parquet_scan('{DATA_DIR}/parquet-testing/g*/t1.parquet')
SELECT COUNT(*) FROM parquet_scan('{DATA_DIR}/parquet-testing/bug1588.parquet') WHERE has_image_link = 1
SELECT * FROM parquet_scan('{DATA_DIR}/parquet-testing/bug1589.parquet')
SELECT * FROM parquet_scan('{DATA_DIR}/parquet-testing/bug2267.parquet')
SELECT typeof(#1) FROM parquet_scan('{DATA_DIR}/parquet-testing/binary_string.parquet',binary_as_string=False) limit 1
SELECT AVG(y), AVG(m), AVG(v), AVG(j) FROM '__TEST_DIR__/orders_ym/**/*.parquet'
select * from parquet_schema('{DATA_DIR}/parquet-testing/glob/*.parquet')
select * from parquet_schema(['{DATA_DIR}/parquet-testing/decimal/int64_decimal.parquet', '{DATA_DIR}/parquet-testing/decimal/int64_decimal.parquet'])
select hex(max(b)) from '__TEST_DIR__/blobs.parquet'
SELECT COUNT(*) FROM '__TEST_DIR__/evolution_*.parquet' WHERE a=2
SELECT typeof(a), typeof(b), typeof(c) FROM parquet_scan('__TEST_DIR__/ubn*.parquet', UNION_BY_NAME=TRUE) LIMIT 1
select file_row_number from '{DATA_DIR}/parquet-testing/glob/t1.parquet' where file_row_number=0
SELECT MIN(first_name), MAX(first_name) FROM userdata1
SELECT FIRST(ip_address) OVER w, LAST(ip_address) OVER w FROM userdata1 WINDOW w AS (ORDER BY id RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) LIMIT 1
SELECT MIN(title), MAX(title) FROM userdata1
select timestamp from '{DATA_DIR}/parquet-testing/issue_5533_timestamp_ms_stats.parquet' where timestamp >= '2022-11-27 00:00:00'
SELECT i, j, k, x, parse_path(filename)[-2] FROM read_parquet('{DATA_DIR}/parquet-testing/hive-partitioning/union_by_name/*/f2.parquet', hive_partitioning=1, union_by_name=1, filename=1) WHERE k>0 ORDER BY j
SELECT typeof(d) FROM '__TEST_DIR__/dates.parquet' LIMIT 1
select field_id from parquet_schema('__TEST_DIR__/my.parquet') where name = 'j'
SELECT * FROM '~/integers.parquet'
select sum(range) = (count(*) * (count(*) - 1)) // 2 from '__TEST_DIR__/parquet_write_memory_usage.parquet'
SELECT * FROM unsigned EXCEPT SELECT * FROM '__TEST_DIR__/unsigned.parquet'
select * from parquet_scan('{DATA_DIR}/parquet-testing/hive-partitioning/duplicate_names/**/*.parquet') ORDER BY ALL
SELECT part_col, value_col, value2_col FROM '__TEST_DIR__/partitioned5/part_col=0/*.parquet' ORDER BY value2_col
SELECT partition, SUM(col1) FILTER (col2%7>2) FROM partitioned_tbl GROUP BY partition ORDER BY ALL
SELECT partition2, SUM(col1) FROM partitioned_tbl2 GROUP BY partition2 ORDER BY ALL
SELECT part_col, value_col, value2_col, value3_col, value4_col, value5_col, value6_col, value7_col, value8_col, value9_col FROM '__TEST_DIR__/no-part-cols7/value9_col=*/value8_col=*/value7_col=*/*.parquet' ORDER BY 1
SELECT part_col, value_col, value2_col FROM '__TEST_DIR__/csv-no-part-cols/part_col=0/*.csv' ORDER BY value2_col
SELECT count(*) FROM '__TEST_DIR__/row_groups_per_file9/*.parquet'
SELECT * FROM db2.test
select currval('backup.main.seq')
WITH a(x) AS MATERIALIZED ( SELECT * FROM generate_series(1, 10) ), b(x) AS MATERIALIZED ( SELECT * FROM a WHERE x < 8 ) SELECT * FROM b WHERE x % 3 = 1 ORDER BY x
WITH RECURSIVE cte(d) AS MATERIALIZED ( SELECT 1 UNION ALL (WITH c(d) AS (SELECT * FROM cte) SELECT d + 1 FROM c WHERE FALSE ) ) SELECT max(d) FROM cte
WITH CTE AS MATERIALIZED ( SELECT A1, * FROM T0 LEFT JOIN ( SELECT C1 AS A1 FROM T1 ) ON T0.C1 = A1 ) SELECT A1 FROM CTE
WITH RECURSIVE fib AS MATERIALIZED ( SELECT 1 AS n, 1::bigint AS "fibₙ", 1::bigint AS "fibₙ₊₁" UNION ALL SELECT n+1, "fibₙ₊₁", "fibₙ" + "fibₙ₊₁" FROM fib WHERE n <= 20 ) SELECT n, "fibₙ" FROM fib LIMIT 20
with recursive t as MATERIALIZED (select 1 as x union all select x+1 from t as m where m.x < 3) select * from t
with recursive t as MATERIALIZED (select 1 as x union select x+(SELECT 1+t.x) from t where x < 5) select * from t order by x
with cte1 as (Select i as j from a) select * from (with cte2 as (select max(j) as j from cte1) select * from cte2) f
with a as (select * from va) select * from a
WITH RECURSIVE cte(x,y) USING KEY (x) AS ( SELECT 1, 0 UNION SELECT x, y+1 FROM cte WHERE y < 10 ) TABLE cte
SELECT * FROM v3 ORDER BY 1
SELECT * FROM s1.table01 ORDER BY i
SELECT * FROM integers WHERE 2>1
SELECT * FROM integers WHERE 2 NOT IN (2, 3, 4, 5)
SELECT * FROM integers WHERE a=2 AND a<4
SELECT * FROM integers WHERE a=4 AND a<2
SELECT * FROM nested_structs WHERE s.a.b < 2
SELECT * FROM vals1 WHERE i>=10 AND j>i
SELECT * FROM vals1 WHERE j>=i AND i=5
SELECT * FROM vals1 WHERE j>=i AND i>=10
SELECT * FROM vals1, vals2 WHERE i<1 AND k<=j AND j<=i AND l<=k
SELECT c0, c1 FROM t_varchar WHERE TRY(c1::INTEGER - c0::INTEGER) IS NULL ORDER BY c0 NULLS LAST
SELECT array_cross_product(array_value(1,2,3), array_value(1.0,5.0,7.0)::${TYPE}[3])
SELECT array_inner_product([1, 1, 1]::${type}[3], [1, 1, 1]::${type}[3])
SELECT list_sort(array_value(3,2,1)) = list_sort([3,2,1])
SELECT suggestion, suggestion_start FROM sql_auto_complete('COP') LIMIT 1
SELECT suggestion, suggestion_start FROM sql_auto_complete('SELECT NULL FR') LIMIT 1
SELECT suggestion, suggestion_start FROM sql_auto_complete('SEL') LIMIT 1
SELECT suggestion, suggestion_start FROM sql_auto_complete('SELECT * FROM tbl OR') LIMIT 1
SELECT suggestion, suggestion_start FROM sql_auto_complete('SELECT (SELECT SUM(l_orderkey) FROM lineit') LIMIT 1
SELECT WEEKDAY(d) FROM dates
SELECT stats(EXTRACT(millennium FROM d)) FROM dates LIMIT 1
SELECT stats(DAYOFMONTH(d)) FROM dates LIMIT 1
SELECT date_part('era', d) FROM dates
select date_part('year', dt::DATE) * 10, from generate_series('2050-01-01'::date,'2051-12-31'::date,interval 1 day) t(dt) where dt = '2050-12-31'
SELECT d, epoch_ns(d) FROM dates WHERE d != '0044-03-15 (BC)' OR d IS NULL ORDER BY ALL
SELECT DATE_PART(['weekday', 'isodow', 'doy', 'julian'], '2022-01-01'::DATE) AS parts
WITH cte AS ( SELECT NULL::VARCHAR part FROM range(1) ) SELECT date_part(part, TIMESTAMP '2019-01-06 04:03:02') FROM cte
SELECT date_trunc(NULL, d) FROM dates
SELECT date_trunc(NULL, d) FROM timestamps LIMIT 3
select date_trunc('hour', '2022-08-15 07:52:55'::${temporal})
SELECT date_trunc(s, d) FROM timestamps
SELECT stats(date_trunc('month', d)) FROM dates LIMIT 1
WITH cte AS ( SELECT NULL::VARCHAR part FROM range(1) ) SELECT date_trunc(part, TIMESTAMP '2019-01-06 04:03:02') FROM cte
SELECT EXTRACT(YEARWEEK FROM i) FROM dates
SELECT EXTRACT(week FROM cast('2007-01-01' AS DATE) + 21)
SELECT EXTRACT(year FROM d) FROM dates2 ORDER BY 1
SELECT strftime(d, '%f') FROM dates ORDER BY d
select d, time_bucket('3 months'::interval, d, null::interval) from dates
select time_bucket('-1 month'::interval, '2022-12-22'::date, null::interval)
select time_bucket('-1 month'::interval, '2022-12-22'::date, null::date)
WITH cte AS ( SELECT NULL::INTERVAL i, NULL::DATE d, NULL::TIMESTAMP t FROM range(1) ) SELECT time_bucket(i, d, i) FROM cte
SELECT enum_first(null::rainbow)
SELECT enum_last(null::rainbow)
SELECT enum_range(null::rainbow)
SELECT * FROM tbl WHERE CASE WHEN i%2=0 THEN 1 ELSE 0 END AND CASE WHEN i<5 THEN 1 ELSE 0 END
SELECT i BETWEEN NULL AND 2 FROM integers ORDER BY i
SELECT count FROM duckdb_connection_count()
SELECT IF(true, '2020-05-05'::date, '1996-11-05 10:11:56'::timestamp), IF(false, '2020-05-05'::date, '1996-11-05 10:11:56'::timestamp), IF(NULL, '2020-05-05'::date, '1996-11-05 10:11:56'::timestamp)
SELECT sleep_ms(100)
SELECT EXTRACT(century FROM i) FROM intervals
SELECT EXTRACT(millisecond FROM i) FROM intervals
SELECT list_any_value(s), list_any_value(l), list_any_value(r) FROM five_complex
SELECT list_sum(i) FROM decimals
SELECT list_histogram(g) from hist_data
select list_kurtosis(k) from aggr
SELECT list_mad(r) FROM date
select list_mad(x) from (values (['23:59:59.999999'::time, '00:00:00'::time])) tbl(x)
SELECT list_max(s), list_max(l), list_max(r) FROM five_complex
select list_mode(v) from aggr
select round(list_stddev_samp(val), 1) from stddev_test
SELECT flatten(NULL)
select flatten(NULL)
SELECT range(1, NULL)
SELECT range(timestamptz '2020-01-01', timestamptz '2020-01-01', interval '1' day)
SELECT list_filter([5, NULL, 7, NULL], x -> x IS NOT NULL)
SELECT list_filter(l, x -> x + 1 <= 2) FROM lists
SELECT list_transform([[1, 3], [2, 3, 1], [2, 4, 2]], x -> list_filter(x, y -> y <= 2))
SELECT g, list_count(list_filter(l, x -> x % 2 = 0)) FROM large_lists ORDER BY g
SELECT list_apply([5,6], x -> list_filter([4,8], y -> y))
SELECT list_intersect([1], list_intersect([1], [1]))
SELECT 1, list_transform([5, 4, 3], x -> x + 1) AS lst GROUP BY 1
SELECT list_filter([[1, 2], NULL, [3], [4, NULL]], f -> list_count(macro_with_lambda(f, 2)) > 1)
SELECT list_transform([[1], [2], [3]], x -> list_concat(list_transform(x, y -> y + 1), list_transform(x, z -> z - 1)))
SELECT array_apply([1, NULL], arr_elem -> arr_elem - 4)
SELECT list_transform([1,2,3,4,5], (x, i) -> (x * 10 / i))
SELECT list_filter([1, NULL, -2, NULL], lambda x: x % 2 != 0)
SELECT tag_product, list_aggr(list_transform( string_split(tag_product, ' '), lambda word: lower(word)), 'string_agg', ',') AS tag_material, FROM tbl GROUP BY tag_product ORDER BY ALL
SELECT list_filter([[1, 2], NULL, [3], [4, NULL]], lambda f: list_count(macro_with_lambda(f, 2)) > 1)
SELECT list_reduce([1, 2, 3], lambda x, y: x * y)
SELECT list_reduce([[10, 20], [30, 40], NULL, [NULL, 60], NULL], lambda x, y: list_pack( list_reduce(x, lambda l, m: l + m) + list_reduce(y, lambda n, o: n + o)))
SELECT list_reduce([1, 2, 3], lambda x, y, i: x + y + i, -1)
SELECT l, [[{'x+y': x + y, 'x': x, 'y': y, 'l': l} for y in [42, 43]] for x in l] FROM no_overwrite
SELECT list_transform(list_value(list_unique(list_concat([1,2],[2,2]))), lambda x: (x + 1)::INTEGER)
select list_transform(bb, lambda x: [x, b]), bb, b from (select list(b) over wind as bb, first(b) over wind as b from test window wind as (order by a asc, b asc rows between 4 preceding and current row) qualify row_number() over wind >4)
SELECT list_transform(['1', '2', '3', '4'], lambda x, i: (x || ' + ' || CAST(i AS string)))
SELECT list_transform([1,2,3,4,5], lambda x, i: (x * 10 / i))
SELECT list_prepend(1, [2, 3])
SELECT list_contains([1,2,3],1.0)
SELECT list_contains([[NULL],[1], [1,2,3]],NULL)
SELECT list_sort(list_distinct(['a', 'b、c', 'a']))
select ${f}(list_intersect(l1, l2), list_intersect(l2, l1)) from tbl
SELECT list_sort(list_intersect([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], [3, 6, 9, 12]))
SELECT list_position([1],NULL)
SELECT list_position([[1,2,3],[1]],[1,2,3])
SELECT list_position([[1,2,3],NULL],NULL)
SELECT list_reverse([1, 42, 2])
SELECT list_reverse([[1], [1, 2], NULL, [NULL], [], [1, 2, 3]])
SELECT list_contains(list_value([1, 2]::INTEGER[2], [3, 4]::INTEGER[2]), [1, 2]::INTEGER[2])
SELECT LIST_VALUE([1, 7], [2], [3], NULL)
SELECT list_value({'a': 1, 'b': 'a'}, {'a': 2, 'b': 'b'})
SELECT list_where([1::${type}], [true])
SELECT list_where(['14:59:37'::TIMETZ], [true])
SELECT list_where(['{a: 1}'::BLOB, '{a: 3}'::BLOB], [true, false])
WITH data AS (SELECT 1 AS a, 2 AS b, 3 AS c) SELECT struct_insert (data, d := 4) FROM data
WITH data AS (SELECT 1 AS a, 2 AS b, 3 AS c) SELECT struct_update (data, d := 4) FROM data
SELECT struct_update(col, i := 10, a := NULL, b := NULL::VARCHAR, c := [NULL]) FROM tbl ORDER BY ALL
SELECT cast(FLOOR(n::float) as bigint) FROM numbers ORDER BY n
SELECT gamma(2::tinyint)
SELECT COUNT(*) FROM (SELECT a FROM t1 JOIN t2 ON (a=b) JOIN t3 ON (b=c)) s1
select round(42.12345::DOUBLE)
select round(42.12345::DOUBLE, 4), round(42.1235::DOUBLE, 1000)
select round(a, b) from roundme
SELECT roundBankers(45.5, 0), roundBankers(44.5, 0)
SELECT roundBankers(-45.5, 0), roundBankers(-44.5, 0)
SELECT cast(TAN(n::tinyint)*1000 as bigint) FROM numbers ORDER BY n
SELECT cast(ASIN(n::bigint)*1000 as bigint) FROM numbers WHERE n between -1 and 1 ORDER BY n
select trunc(15::${unsigned})
select trunc(0::${datatype}, 2), trunc(0::${datatype}, -1)
SELECT true OR true
SELECT instr(v, chr(0)) FROM null_byte
SELECT * FROM null_byte WHERE regexp_matches(v, chr(0))
select parse_path('\path\to\file', 'forward_slash')
select parse_path('/')
SELECT parse_dirname('\', 'backslash')
SELECT parse_dirname('wh@t3ve%\42/12 ch,ars.sth', 'both_slash')
SELECT * FROM (VALUES (parse_filename('path/to/file.csv', 'system')), (parse_filename('path/to/file.csv\file2.csv', 'both_slash')), (parse_filename('path/to/file.csv', 'forward_slash')), (parse_filename('path\to\file.csv/file2.csv', 'backslash'))) tbl(i)
SELECT parse_filename('/path/to/file.csv\file2.csv', true)
SELECT parse_filename('path/to/file.csv', NULL, NULL)
SELECT parse_filename('')
SELECT regexp_escape('@')
select regexp_extract_all('abc=111, def=222, ghi=333', '("[^"]+"|\w+)=("[^"]+"|\w+)', 1)
select regexp_extract_all('abc', '.')
select regexp_extract_all('щцф', '.{3}')
SELECT str, REGEXP_EXTRACT_ALL(str,'ab?cd') AS matched FROM ( VALUES ('acd'), ('abcd'), ('abcdacd'), ('abbcd'), ('abbbcd'), ('ab1cd') ) AS t(str)
SELECT regexp_extract_all('foobarbaz', '((BA)([RZ]))', ['whole','ba','letter'], 'i') AS res
SELECT regexp_extract_all('abc=111, def=222, ghi=333', '("[^"]+"|\w+)=("[^"]+"|\w+)', ['key','val']) AS res
SELECT regexp_extract_all('abcd', 'a(bc)*d', ['g1']) AS res
select bar(1, 0, 'nan'::double)
select bar(10, 10, 10, 10)
SELECT UPPER('Αα Ββ Γγ Δδ Εε Ζζ Ηη Θθ Ιι Κκ Λλ Μμ Νν Ξξ Οο Ππ Ρρ Σσς Ττ Υυ Φφ Χχ Ψψ Ωω'), LOWER('Αα Ββ Γγ Δδ Εε Ζζ Ηη Θθ Ιι Κκ Λλ Μμ Νν Ξξ Οο Ππ Ρρ Σσς Ττ Υυ Φφ Χχ Ψψ Ωω')
SELECT substring_grapheme('test: 🤦🏼‍♂️hello🤦🏼‍♂️ world', 7, 7)
SELECT s || ' ' || '🦆' FROM strings ORDER BY s
select list_concat([1], NULL)
select list_concat([1], [2], [3])
select CONCAT(a, 'SUFFIX') FROM strings
select CONCAT('1234567890', '1234567890'), CONCAT('1234567890', NULL)
SELECT damerau_levenshtein(NULL, '')
SELECT damerau_levenshtein(s, '') FROM strings
SELECT format('{}', 'hello', 'world')
SELECT 'aaa' GLOB 'bbb'
SELECT 'aaa' GLOB '*b'
SELECT 'ababac' ILIKE '%%%a%%%b%%a%b%%%%%a%c%%'
SELECT s FROM strings WHERE 'aba' ILIKE pat
SELECT instr(s,'d') FROM strings
SELECT instr(s,'he-man') FROM strings
SELECT round(jaccard('ab', 'aabbcc'), 3)
SELECT round(jaccard('aabbccddeeff', 'ab'), 3)
SELECT levenshtein('lawn', 'flaw')
SELECT levenshtein('hi', s) from strings
SELECT hamming('hallo', 'hallo')
select LPAD('MotörHead', 16, 'RÄcks'), LPAD('MotörHead', 12, 'RÄcks'), LPAD('MotörHead', 10, 'RÄcks')
SELECT prefix('two ñ three ₡ four 🦆 end', 'two ñ three ₡ four 🦆 end')
SELECT printf('floats: %4.2f %+.0e %E', 3.1416, 3.1416, 3.1416)
SELECT printf('%s: %s', pstring, pstring) FROM strings ORDER BY idx
select REVERSE(''), REVERSE('Hello'), REVERSE('MotörHead'), REVERSE(NULL)
select split_part('a,b,c',',',5)
SELECT starts_with(NULL,NULL) FROM strings
SELECT 'hello world' ^@ 'a', 'hello world' ^@ 'ha', 'hello world' ^@ 'hea', 'hello world' ^@ 'hela', 'hello world' ^@ 'hella', 'hello world' ^@ 'helloa', 'hello world' ^@ 'hello a', 'hello world' ^@ 'hello wa', 'hello world' ^@ 'hello woa', 'hello world' ^@ 'hello wora'
SELECT s ^@ 'olá mundo' FROM strings
SELECT array_slice(s, 0, length) FROM strings
SELECT 'hello'[NULL:length+NULL] FROM strings
SELECT n[NULL:NULL+NULL] FROM strings, nulltable
SELECT substring('hello' from NULL for length) FROM strings
SELECT ${FUN}('abc', INSTR('abc', 'b'))
SELECT suffix('ñeft', 'ñeft')
select TRANSLATE(a, 'loD', '🦆') FROM strings
select TRIM(''), TRIM('Neither'), TRIM(' Leading'), TRIM('Trailing '), TRIM(' Both '), TRIM(NULL), TRIM(' ')
SELECT EXTRACT(second FROM i) FROM times
SELECT AGE(NULL, NULL)
SELECT ts, DATE_PART(['hour', 'minute', 'microsecond'], ts) AS parts FROM timestamps ORDER BY 1
select '2000-04-09 17:00:00-07'::timestamptz - interval 2400 hours
select '-infinity'::timestamptz - '1 day'::interval
SELECT origin + (dst2 - origin) FROM london
SELECT starttime, recordtime, date_diff('minute', starttime, recordtime) FROM issue9673
SELECT DATE_PART('timezone_minute', ts) FROM timestamps
SELECT DATE_PART('timezone', '2021-07-31 00:00:00-07'::TIMESTAMPTZ)
SELECT date_trunc('microseconds', TIMESTAMPTZ '2019-01-06 04:03:02.123456-08')
select bool_and( date_part('epoch', time_bucket(interval '3' day, timestamptz '2023-06-07 16:08:09+00', origin)) = date_part('epoch', time_bucket(interval '3' day, timestamp '2023-06-07 16:08:09', origin at time zone 'UTC')) ) from generate_series(timestamptz '2023-01-03 00:00:00+05', timestamptz '2024-01-04 00:00:00+05', interval '7877' minute) as t(origin)
SELECT strftime(d, '%I') FROM timestamps ORDER BY d
SELECT strptime('30', '%W'), strftime('1900-07-23'::DATE, '%W')
select strptime('-infinity', '%m/%d/%Y')
select strptime('epoch', '%m/%d/%Y')
select t, time_bucket('3 days'::interval, t) from timestamps
select time_bucket('1 microseconds'::interval, '294247-01-10 04:00:54.775806'::timestamp)
SELECT try_strptime('21 June, 2018', '%d %B, %Y')
SELECT try_strptime('20182010', '%Y%d%m')
SELECT try_strptime('2021-19-4', '%G-%V-%u'), strftime('2021-05-13'::DATE, '%G-%V-%u')
SELECT try_strptime('2021-0-5', '%Y-%W-%w'), strftime('2021-01-01'::DATE, '%Y-%W-%w')
SELECT try_strptime('969-10-10', '%y-%m-%d')
SELECT try_strptime('2021 19', '%G %W')
select TRY_CAST(col.almost_a_number AS BIGINT) from tbl order by all
SELECT a, b, gen, c, d FROM tbl_comp WHERE c = 1
SELECT * FROM integers WHERE i > 1
SELECT i FROM tbl WHERE i = 60001
SELECT CASE WHEN ( CASE WHEN get_block_size('test_art_import') = 16384 THEN used_blocks < 3 ELSE used_blocks < 2 END ) THEN NULL ELSE T END FROM pragma_database_size() T
SELECT id FROM tbl_deser_scan WHERE id >= 424242
SELECT k FROM integers WHERE k >= 100000::INTEGER ORDER BY k
SELECT COUNT(i) FROM strings WHERE i >= 'somesuperbigstring' and i <='somesuperbigstringz'
select a.seq_no, a.amount, b.amount from issue13899 as a asof join issue13899 as b on a.seq_no>=b.seq_no and b.amount is not null ORDER BY 1
SELECT integers.*, integers2_empty.* FROM integers FULL OUTER JOIN integers2_empty USING (i)
SELECT COUNT(*) FROM bigtbl JOIN smalltbl ON (bigtbl.i BETWEEN low AND high AND bigtbl.i IS NOT DISTINCT FROM high - low)
with joined as ( select lhs.k l, rhs.k r from states lhs inner join states rhs on lhs.b < rhs.e and rhs.b < lhs.e and lhs.k = rhs.k ) select count(*) from joined
SELECT t1.x, t2.x FROM 'test/sql/join/iejoin/overlap.left.csv' t1, 'test/sql/join/iejoin/overlap.right.csv' t2 WHERE t1.y > t2.y AND t1.x < t2.x
SELECT test.a, b, c FROM test, test2 WHERE test.a = test2.a AND test.b <= test2.c ORDER BY test.a
SELECT a, (SELECT test.a), c FROM test, test2 WHERE test.b = test2.b ORDER BY c
SELECT COUNT(*) FROM test2
SELECT x.col1, y.col1 FROM tbl_s x JOIN tbl_s y ON x.col0 = y.col0 AND (x.col1 IS DISTINCT FROM y.col1) ORDER BY x.col1
SELECT x.col1, y.col1 FROM tbl_s x JOIN tbl_s y ON x.col0 = y.col0 AND x.col1 != y.col1 ORDER BY x.col1
SELECT x.col1, y.col1 FROM tbl_l x JOIN tbl_l y ON x.col0 = y.col0 AND x.col1 != y.col1 ORDER BY x.col1
SELECT x.col1, y.col1 FROM tbl_s x JOIN tbl_s y ON x.col0 = y.col0 AND (x.col1 IS NOT DISTINCT FROM y.col1) ORDER BY x.col1
SELECT t2.a, t2.b, t2.c FROM t1 JOIN t2 USING(a) ORDER BY t2.b
select * from range(1) tbl(i) left join range(2) tbl2(j) on (i=j) where j+random()<0
SELECT t1.a, t1.b, t2.c FROM t1 NATURAL JOIN t2
select (select * from (select 42) tbl(a) natural join (select 42) tbl2(a))
SELECT DISTINCT sqlancer_v0.c1, sqlancer_t0.rowid FROM sqlancer_v0 NATURAL FULL JOIN sqlancer_t0 WHERE sqlancer_t0.c0 UNION SELECT DISTINCT sqlancer_v0.c1, sqlancer_t0.rowid FROM sqlancer_v0 NATURAL FULL JOIN sqlancer_t0 WHERE (NOT sqlancer_t0.c0) UNION SELECT DISTINCT sqlancer_v0.c1, sqlancer_t0.rowid FROM sqlancer_v0 NATURAL FULL JOIN sqlancer_t0 WHERE ((sqlancer_t0.c0) IS NULL) ORDER BY 2 ASC
select * from t1 anti join t2 on t1.a < t2.b and t1.b < t2.b order by all
SELECT * FROM left_table SEMI JOIN right_table ON (left_table.a = right_table.a)
SELECT * FROM wide
SELECT i, z FROM wide, limits WHERE z BETWEEN c8 AND c9 ORDER BY 1, 2
select lhs.*, rhs.* from list_int lhs, list_int rhs where lhs.i2 = rhs.i2 and lhs.l3 <> rhs.l3 order by lhs.i, rhs.i
SELECT * FROM read_json_auto('{DATA_DIR}/json/arr.json', columns={'v':'VARCHAR','k':'VARCHAR'}, ignore_errors=true)
select typeof(test) from '__TEST_DIR__/manynulls.json' limit 1
WITH path AS ( SELECT 'Status / SubStatus' p ) SELECT '{"Status / SubStatus": "test"}' -> p FROM path
SELECT '{"Status / SubStatus": "test"}' -> '$."Status / SubStatus"'
select id, typeof(id) from '__TEST_DIR__/issue16684.json'
select cast(json('[1,2]') as json[])
select json_contains('{"a": {"b": [{"c": 1, "d": 2}]}}', '[{"d": 2, "c": 1}]')
select json_object('nested', [{duck: 42}, NULL])
select json_object('nested', {nested2: [1, 2, 3]})
select json_quote(struct_pack(a := a, b := b, c := c, d := d, e := e)) from test
SELECT json_array( -9223372036854775808,9223372036854775807,0,1,-1, 0.0, 1.0, -1.0, -1e99, +2e100, 'one','two','three', 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, NULL, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 'abcdefghijklmnopqrstuvwyxzABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwyxzABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwyxzABCDEFGHIJKLMNOPQRSTUVWXYZ', 99)
SELECT json_structure('{"duck":"goose"}'->'duck')
SELECT json_extract(j, '$.b[#-4]'), json_extract(j, '$.b[-4]') FROM t1
SELECT json_extract(j, ['$.b[0]', '$.b[#-1]']) a, a = json_extract(j, ['$.b[0]', '$.b[-1]']) FROM t1
SELECT json_merge_patch('{"a": {"b": 1}}', '{"a": {"b": 2}}')
select json_merge_patch(NULL, '3')
select json_extract(j, '$.my_field.my_nested_field.3') from test
select json_transform('true', '"VARCHAR"')
select json_transform('42', '"DECIMAL(3,1)"')
select json_transform('{}', '"DECIMAL(2,1)"')
select json_transform('{"test": ["a","b"]}', '{"test": "test_enum[]"}')
select json_type('"other"')
SELECT json_type('{"a":[2,3.5,true,false,null,"x"]}')
SELECT json_valid('" \+ "')
SELECT json_valid('" \< "')
SELECT json_valid('" \> "')
SELECT json_valid('" \G "')
SELECT json_valid('" \M "')
SELECT json_valid('{"x":-0.1}')
SELECT json_valid('{"x":01.5}')
SELECT json_value('{"a":2,"c":[4,5,{"f":7}]}', '$.c[2]')
select *, parse_filename(filename) from read_json_auto('{DATA_DIR}/json/example_*.ndjson') order by all
SELECT count(*) from read_ndjson_objects('{DATA_DIR}/json/example_*n.ndjson')
SELECT duck.goose FROM '__TEST_DIR__/nested.json'
select json_group_array(v) from t1
SELECT json_serialize_plan('SELECT *, 1 + 2 FROM tbl1', skip_null := true, skip_empty := true, optimize := true)
select json_serialize_plan('select blob ''\\x00''')
SELECT * FROM integers LIMIT 5 OFFSET 500000
SELECT i FROM integers WHERE (i=1 AND i>0) OR (i=1 AND i<3) ORDER BY i
SELECT max(distinct x) from range(10) tbl(x)
SELECT * FROM vals WHERE 2::${utype}-v=3::${utype}
SELECT a // 0 FROM test
select timestamp_str, cast(timestamp_str as timestamp) from table1 where cast(timestamp_str as timestamp) > cast('2024-05-03T01:00:00+00:00' as timestamp)
SELECT * FROM (SELECT * FROM vals1, vals2) tbl1 LEFT OUTER JOIN (SELECT * FROM vals1, vals2 WHERE i=5 AND k=10) tbl2 ON tbl1.i=tbl2.i AND tbl1.k=tbl2.k WHERE tbl1.i=5 AND tbl1.k=10
SELECT * FROM (SELECT * FROM vals1, vals2) tbl1 LEFT OUTER JOIN (SELECT * FROM vals1, vals2) tbl2 ON tbl1.i=tbl2.i AND tbl1.k=tbl2.k WHERE tbl1.i=5 AND tbl1.k=10
SELECT * FROM t1 where rowid = 6 OR rowid = 9 ORDER BY ALL
SELECT * FROM integers ORDER BY i NULLS FIRST
SELECT my_range, my_ordinality FROM range(3) WITH ORDINALITY AS _(my_range, my_ordinality) ORDER BY my_range,my_ordinality
SELECT 251658240::BIGINT * 251658240::BIGINT
SELECT 100::TINYINT + 1::TINYINT
SELECT 0::TINYINT - 127::TINYINT
SELECT SUM(i) FROM (VALUES (1e308), (1e308)) tbl(i)
SELECT "a42aa", "b84bb", "c126cc" FROM (SELECT MIN(COLUMNS('([a-z])(\d+)')) AS "\1\2\1\1" FROM numerics)
SELECT 6 + 1 // 2
SELECT v.split(' ') FROM varchars
SELECT COLUMNS([x for x in (* EXCLUDE (i))]) FROM integers
VALUES (42,)
VALUES (42,),
SELECT a, b, SUM(c) FROM tbl GROUP BY GROUPING SETS (a, (b, ))
SELECT * FROM pg_settings
SELECT pg_catalog.format_pg_type('DECIMAL', 'test')
SELECT CURRENT_USER
SELECT 1, 2, 3, current_query()
SELECT * FROM (SELECT product, sales, quarter FROM Produce) PIVOT(SUM(sales) FOR quarter IN ('Q1', 'Q2', 'Q3')) ORDER BY ALL
SELECT * FROM monthly_sales PIVOT(SUM(amount) FOR MONTH IN ('JAN', 'FEB', 'MAR', 'APR')) AS p (EMP_ID_renamed, JAN, FEB, MAR, APR) ORDER BY EMP_ID_renamed
SELECT "0_sum(""range"")", "0_sum(""range"")_1" FROM ( PIVOT (FROM range(21)) ON range USING sum(range), sum(range) )
SELECT CURRENT_SETTING('profiling_coverage')
SELECT OPERATOR_CARDINALITY, OPERATOR_NAME, OPERATOR_ROWS_SCANNED, OPERATOR_TIMING, OPERATOR_TYPE FROM ( SELECT unnest(children, max_depth := 2) FROM metrics_output )
SELECT name, file FROM pragma_database_list
SELECT integers.* EXCLUDE (j) FROM integers
SELECT r1, r2 FROM (SELECT * RENAME (integers.i AS r1, j AS r2,) FROM integers)
SELECT * FROM intest WHERE a IN (42, 43)
SELECT rowid+1 FROM a WHERE CASE WHEN i=42 THEN rowid=0 ELSE rowid=1 END
SELECT SUM(rowid), MIN(rowid), MAX(rowid), COUNT(rowid), LAST(rowid) FROM a
SELECT cast(3 AS VARCHAR)
SELECT a3, b3, c3 IN (1, 200) FROM table3
select count(*) from duckdb_table_sample('integral_samples') where d NOT null
SELECT t.t.t.t.t.t.t.t FROM t.t
SELECT s1.t1.t FROM s1.t1, s2.t1
select s.b.col1 from s.b
SELECT 42 WHERE 1=0 UNION ALL SELECT 42
SELECT 1 AS three UNION SELECT 2 UNION SELECT 3 ORDER BY 1
SELECT f1 AS ten FROM FLOAT8_TBL UNION ALL SELECT f1 FROM FLOAT8_TBL
SELECT q2 FROM int8_tbl EXCEPT ALL SELECT DISTINCT q1 FROM int8_tbl ORDER BY 1
SELECT 1 UNION ALL BY NAME SELECT * FROM range(2, 100) UNION ALL BY NAME SELECT 999 LIMIT 5
SELECT x, y FROM t1 UNION ALL BY NAME SELECT y, z FROM t2 ORDER BY z DESC
SELECT 1, a FROM test UNION SELECT b AS a, 1 FROM test2 ORDER BY a, 1
SELECT x FROM t1 UNION BY NAME SELECT x FROM t1 ORDER BY t1.x
SELECT [{'a': 42}, {'b': 84}]
SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT NULL UNION ALL SELECT 3
SELECT 1 UNION (SELECT 1 UNION SELECT 1 UNION SELECT 1)
SELECT COUNT(*) FROM range(1) UNION SELECT COUNT(*) FROM range(2) ORDER BY 1
SELECT 1 UNION ALL SELECT * FROM range(2, 100) UNION ALL SELECT 999 LIMIT 5
SELECT value FROM duckdb_settings() WHERE name = 'block_allocator_memory'
SELECT column_name, key FROM (DESCRIBE SELECT c4 FROM integers WHERE c1=42)
SELECT a FROM vtest ORDER BY a
SELECT MIN(i), MAX(i), COUNT(i), COUNT(*) FROM vals
SELECT compression FROM pragma_storage_info('test') WHERE segment_type ILIKE 'HUGEINT'
SELECT compression FROM pragma_storage_info('test') WHERE segment_type ILIKE 'UHUGEINT'
SELECT compression FROM pragma_storage_info('test_constant') WHERE segment_type ILIKE 'INTEGER' LIMIT 1
select distinct on (types) vector_type(a) as types from test order by all
SELECT * FROM tbl WHERE id >= 5020 AND rle_val=100
SELECT COUNT(*) FROM test WHERE a IS false
SELECT lower(compression)='${compression}' FROM pragma_storage_info('test_empty') WHERE segment_type ILIKE 'VARCHAR' LIMIT 1
SELECT SUM(i)=${i} FROM little_tbl
SELECT MIN(v), MAX(v) FROM null_byte
SELECT total_blocks * block_size < 10 * 262144 FROM pragma_database_size()
SELECT SUM(a) + SUM(b) FROM test
SELECT current_setting('max_temp_directory_size')
select COUNT(d) != 0 from t
SELECT COUNT(*) FROM test WHERE a>0 AND b IS NULL
SELECT NULL > ALL(SELECT * FROM integers)
SELECT ROW(2, 0) > ANY(SELECT 1, 0)
SELECT (0, 0, 0) < ANY(SELECT 1, 0, 0)
SELECT (1, 1) >= ANY(SELECT 1, 1)
SELECT i FROM integers WHERE i < ALL(SELECT MAX(i) FROM integers) ORDER BY 1
SELECT (SELECT MAX(i) FROM integers) AS k, SUM(i) FROM integers GROUP BY k
SELECT i, i <> ANY(SELECT i FROM integers WHERE i>2 OR i IS NULL) FROM integers ORDER BY i
SELECT t0.c2 FROM t0 WHERE NOT EXISTS ( SELECT 1 FROM ( SELECT t0.c2 AS col0 FROM t0 ) AS subQuery WHERE ((t0.c2) IS DISTINCT FROM (subQuery.col0)) )
SELECT EXISTS(SELECT * FROM integers), EXISTS(SELECT * FROM integers)
select * from (select 42 union all select 84) t(a), (select t.a + 1) ORDER BY ALL
SELECT i, (SELECT s1.i FROM (SELECT * FROM integers WHERE i=i1.i) s1) AS j FROM integers i1 ORDER BY i
SELECT i, (SELECT s1.i FROM integers s1 INNER JOIN integers s2 ON s1.i=s2.i AND s1.i=4-i1.i) AS j FROM integers i1 ORDER BY i
SELECT i, (SELECT SUM(x) FROM (SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT i1.i) t(x)) FROM integers i1 ORDER BY i
SELECT i FROM integers i1 ORDER BY (SELECT 100-i1.i)
select (select val + i from generate_series(1, 2, 1) t(i) offset 1) from (select 42 val) t
SELECT i, EXISTS(WITH i2 AS (SELECT i FROM integers WHERE 1=0 AND i1.i=i) SELECT i FROM i2) AS j FROM integers i1 ORDER BY i
SELECT i, (WITH i2 AS (SELECT 42 WHERE i1.i>2) SELECT * FROM i2) AS j FROM integers i1 ORDER BY i
SELECT i, (WITH i2 AS (SELECT 42 WHERE i1.i IS NULL) SELECT * FROM i2) AS j FROM integers i1 ORDER BY i
SELECT i, (SELECT i FROM integers i2 WHERE i-2=(SELECT COUNT(*) FROM integers i2 WHERE i2.i>i1.i)) FROM integers i1 ORDER BY 1
SELECT ( SELECT NULL FROM ( SELECT fuel_type, location_country FROM "t1" WHERE "fuel_type" IS NOT DISTINCT FROM "__input.fuel" LIMIT 1 ) t1) FROM t2 AS __p
SELECT c-(SELECT sum(c) FROM t1) FROM t1
SELECT 'bla' IN (SELECT * FROM strings WHERE v=s1.v) FROM strings s1 ORDER BY v
SELECT * FROM strings s1 WHERE EXISTS(SELECT v FROM strings WHERE v=s1.v) ORDER BY v
select * from (select i as j from a group by i) sq1 where j = 42
select * from (select 42) sq1 union all select * from (select 43) sq2
SELECT * FROM (VALUES (42, 43))
SELECT unnamed_subquery.a, unnamed_subquery2.b FROM (SELECT 42 a), (SELECT 43 b)
SELECT table_name, constraint_index, constraint_type, UNNEST(constraint_column_names) col_name FROM duckdb_constraints ORDER BY table_name, constraint_index, col_name
SELECT name FROM duckdb_optimizers() WHERE name='join_order'
select constraint_name, unique_constraint_name from information_schema.referential_constraints
select * from generate_series(4,19,5)a
SELECT * FROM (SELECT DATE '2000-01-01', DATE '2000-10-1', NULL) t(s, e, increment), generate_series(s, e, increment) t2(j) ORDER BY s, j
select count(*) from (values (1), (10), (100), (1000), (10000)) t(a), range(a)
SELECT d::DATE FROM generate_series(DATE '1992-01-01', DATE '1992-10-01', INTERVAL (1) MONTH) tbl(d)
SELECT * FROM repeat(blob '\x00\x00hello', 2)
select cast('2020-01-01T15:00:00+0000'::timestamptz as timestamp)
SELECT '2001-02-16 20:38:40'::TIMESTAMP AT TIME ZONE 'America/Denver'
SELECT '2001-02-16 20:38:40'::TIMESTAMP AT TIME ZONE NULL
SELECT ts AT TIME ZONE tz, tstz AT TIME ZONE tz, ttz AT TIME ZONE tz FROM attimezone
SELECT * FROM integers ORDER BY i DESC NULLS FIRST LIMIT 2
SELECT MAX(i) FROM integers
select tinyint::varchar::bignum = tinyint::bignum, smallint::varchar::bignum = smallint::bignum, int::varchar::bignum = int::bignum, bigint::varchar::bignum = bigint::bignum, hugeint::varchar::bignum = hugeint::bignum, uhugeint::varchar::bignum = uhugeint::bignum, utinyint::varchar::bignum = utinyint::bignum, usmallint::varchar::bignum = usmallint::bignum, uint::varchar::bignum = uint::bignum, ubigint::varchar::bignum = ubigint::bignum FROM test_all_types() where tinyint is not null
select (-1.7976931348623157E+308)::double::bignum = '-179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368'::bignum
select '-1000000000000000'::bignum::double
select (-1000)::BIGNUM + (-1000)::BIGNUM
SELECT (-500)::BIGNUM + (1000::BIGNUM + ((-250)::BIGNUM))
select count(*) from integers where a < 0::BIGNUM
select '0010.9'::BIGNUM
SELECT bitstring('0101011'::${cast}, 203)
SELECT bit_count(b) FROM bits
SELECT TRY_CAST('\\b12' AS BLOB)
SELECT '1992-02-29'::DATE::VARCHAR == '1992-02-29'
SELECT '-1000-01-01'::DATE::VARCHAR == '1001-01-01 (BC)'
SELECT '1992-02-29'::DATE::VARCHAR
select try_cast('5881580-07-10' as date)
SELECT 1.25::DOUBLE::DECIMAL(3,2)
SELECT MAX(NULL::DECIMAL), MAX('0.1'::DECIMAL(4,1))::VARCHAR, MAX('4938245.1'::DECIMAL(9,1))::VARCHAR, MAX('45672564564938245.1'::DECIMAL(18,1))::VARCHAR, MAX('4567645908450368043562342564564938245.1'::DECIMAL(38,1))::VARCHAR
SELECT '-1e3'::DECIMAL, '-0.1e3'::DECIMAL, '-.1e-1'::DECIMAL, '-0.1e-1'::DECIMAL
SELECT TRY_CAST('100000000000000000000'::DOUBLE AS DECIMAL(20,0))
SELECT typeof(42.), typeof(42e3), typeof(4.23e1), typeof(10e20), typeof(.34), typeof(-2.3), typeof(10e100)
SELECT CEIL('999.9'::DECIMAL(4,1)), CEIL('99999999.9'::DECIMAL(9,1)), CEIL('99999999999999999.9'::DECIMAL(18,1)), CEIL('9999999999999999999999999999999999999.9'::DECIMAL(38,1))
select ['happy']::mood[]
SELECT person_mood in ('sad') FROM person_pet_den
select count(*) from t1, t2 where t1.a::VARCHAR > t2.b
SELECT f FROM floats WHERE f>1 ORDER BY 1
SELECT f, SUM(i) FROM floats GROUP BY f ORDER BY f
SELECT '${val}'::${type} + 'inf'::${type}
SELECT '${val}'::${type} % 'inf'::${type}
SELECT 'nan'::${source_type}::${target_type}
select ${unary_func}('nan'::${type})
SELECT 8589934592::HUGEINT * 19807040628566084398385987583::HUGEINT
SELECT 1 - (-170141183460469231731687303715884105724)
SELECT 170141183460469231731687303715884105727 // 170141183460469231731687303715884105727
SELECT 32767::HUGEINT::SMALLINT, -32767::HUGEINT::SMALLINT
select floor(1::HUGEINT), floor('-1329227995784915872903807060280344576'::HUGEINT), floor(0::HUGEINT)
SELECT interval (i + 1) day from range(1, 4) tbl(i)
SELECT '1.5 SECOND'::INTERVAL
SELECT '-1.5 SECOND'::INTERVAL
select ${func}(84::${type})
SELECT try_cast('00:00:' as interval)
SELECT INTERVAL '2 years'::VARCHAR
SELECT INTERVAL '-2Y 4 days 5 Hours 1 MinUteS 3S 20mS 16uS'
select interval '-05:12:34.567890' as test_interval
SELECT INTERVAL '1 millennium 2 centuries 1 decade 3 quarter'
SELECT DATE '1992-03-01' - INTERVAL '7' MONTH
SELECT l <= r FROM list_int1
SELECT [1] >= [1, 2]
SELECT [] < []
SELECT l <> r FROM list_str
SELECT NULL >= [{'x': 'duck', 'y': 1}]
SELECT STATS([interval 1 year, interval 2 year])
SELECT UNNEST([[1, 2, 3]], recursive := true)
SELECT name, UNNEST(address), UNNEST([1]) FROM people
SELECT * FROM UNNEST(ARRAY[1, 2, 3])
SELECT i FROM UNNEST([]::INT[]) AS tbl(i)
SELECT MAP(['category', 'min', 'max'], [category, MIN(score), MAX(score)]) FROM groups GROUP BY category ORDER BY ALL
SELECT TRY_CAST(x as INT[2][2]) FROM (VALUES ([[1,2],[3,4]]), ([[5,6],[7,8],[9,10]])) AS t(x)
SELECT 1=ANY(l) FROM v1
SELECT ARRAY[1, i] FROM range(3) tbl(i) ORDER BY i
SELECT (SELECT CASE WHEN 1=0 THEN LIST_VALUE() ELSE NULL END)
SELECT LIST_EXTRACT(LIST_VALUE(42::BIGINT), 1)
SELECT list_extract('1', 0)
SELECT list_aggregate(str, 'count') FROM struct_data
SELECT n[:] FROM lists, nulltable
SELECT arr, list_concat(arr[1:-:2], arr) FROM (SELECT [1,2,3,4,5]) AS _(arr)
SELECT ([1,2,3,4,5,6])[1:9223372036854775807]
SELECT a[start:stop:step] FROM null_tbl
SELECT g, LIST(e/2.0) from list_data GROUP BY g order by g
select col[123] from tinyint_key
select * from real_key
SELECT MAP_FROM_ENTRIES(NULL)
SELECT MAP_FROM_ENTRIES(input) FROM tbl
SELECT MAP(list_value(), list_value())
SELECT list_transform(l, lambda x: {'map1': MAP {x::VARCHAR:1::VARCHAR, 'b'::VARCHAR: x::VARCHAR}}) FROM i
select CARDINALITY(MAP())
select a, cardinality(m) from (select a,MAP(lsta,lstb) as m from (SELECT list(a) as lsta, list(b) as lstb, a FROM ints group by a) as lst_tbl) as T ORDER BY ALL
select cardinality(m) from (select MAP(list_value(1), list_value(2)) from range(5) tbl(i)) tbl(m)
SELECT map_entries(map([5], [NULL]))
select MAP_ENTRIES(MAP(NULL, NULL))
select map_keys(MAP(['a'],[5]))
select list_apply(maps, lambda x: map_keys(x)) from tbl
select map_keys_macro(map_from_entries(list)) from t1
select m from (select MAP(lsta,lstb) as m from (SELECT list(a) as lsta, list(b) as lstb FROM ints where a < 4 and b > 1) as lst_tbl) as T
select MAP_VALUES(NULL::MAP(INT, BIGINT))
select min(struct_pack(i := i, j := i + 2)), max(struct_pack(i := i, j := i + 2)), first(struct_pack(i := i, j := i + 2)) from range(10) tbl(i)
SELECT * FROM t2 where id>=4 order by id
SELECT e, STRUCT_EXTRACT(STRUCT_PACK(xx := e//2), 'xx')*2 as s FROM struct_data WHERE e > 4
SELECT STRUCT_EXTRACT(STRUCT_PACK(a := 42, b := 43), 'a') s
SELECT a FROM test
SELECT a + b FROM test
SELECT TRY_CAST(i AS DECIMAL(3,0))::BIGINT FROM bigints ORDER BY i
select typeof([100::USMALLINT, 10000::SMALLINT])
SELECT TRY_CAST(i AS UINTEGER) FROM hugeints ORDER BY i
SELECT i::UBIGINT FROM integers WHERE i>=0 ORDER BY i
SELECT TRY_CAST(i AS USMALLINT)::INTEGER FROM integers ORDER BY i
SELECT i::BOOL FROM integers ORDER BY i
SELECT i::USMALLINT FROM smallints WHERE i>=0 ORDER BY i
SELECT i::UINTEGER::TINYINT FROM tinyints WHERE i>=0 ORDER BY i
SELECT i::HUGEINT::TINYINT FROM tinyints ORDER BY i
SELECT typeof(1::UBIGINT + 1)
SELECT i::BOOL FROM ubigints ORDER BY i
SELECT i::DOUBLE FROM uhugeints ORDER BY i
SELECT TRY_CAST(i AS USMALLINT) FROM uintegers ORDER BY i
SELECT TRY_CAST(i AS TINYINT) FROM uintegers ORDER BY i
SELECT i::DECIMAL(38,0)::UINTEGER FROM uintegers ORDER BY i
SELECT s.nested_struct.a FROM ${source}
SELECT s.name.v FROM ${source} WHERE s.nested_struct.b
SELECT CASE WHEN 1=1 THEN NULL ELSE {'i': 2} END
SELECT l >= r FROM struct_str
SELECT NULL <> {'x': 'duck', 'y': 1}
SELECT {'x': 'duck', 'y': 1} <> NULL
SELECT {'x': 'duck', 'y': 1} > NULL
SELECT l <= r FROM struct_str_int
SELECT NULL < {'x': 1, 'y': {'a': 'duck', 'b': 1.5}}
SELECT {'x': 1, 'y': ['duck', 'somateria']} = NULL
SELECT {'x': 1, 'y': ['duck', 'somateria']} >= {'x': 1, 'y': ['duck', 'somateria']}
select {'x': a, 'y': a+1, 'z': a+2}>{'x': 1, 'y': 2, 'z': 3} from range(5) tbl(a)
SELECT struct_contains(ROW([1, 2], 3), [1, 2])
SELECT struct_contains(ROW(ROW(1, 2), [1,2]), ROW(5, 6))
SELECT l IS DISTINCT FROM r FROM struct_str_int
SELECT struct_position(ROW([1, NULL], [1], [1, 2, 3]), [1, NULL])
SELECT s['b'] FROM ${source}
select row(42, 'hello') union all select '(84, world)'
select count(*) from times inner join timestamp on (timestamp.i::TIME = times.i)
SELECT '15:30:00.123456789'::TIME_NS
SELECT '02:30:00+1200'::TIMETZ
select time '23:59:59.999999' + interval (1) second
select try_cast('23:59:60' as time)
SELECT DATE '1992-01-01'::TIMESTAMP_MS
SELECT '-1000-01-01 01:03:20.45432'::TIMESTAMP::VARCHAR
SELECT QUANTILE_CONT(ts, 0.25), QUANTILE_CONT(tstz, 0.25), QUANTILE_CONT(dt, 0.25) FROM specials
SELECT timestamp ' 2017-07-23 13:10:11 '
SELECT MAX(t) FROM timestamp
SELECT YEAR(TIMESTAMP '1992-01-01 01:01:01')
select count(*) from timestamp2 inner join timestamp1 on (timestamp1.i = timestamp2.i)
SELECT typeof([TIMESTAMP_NS '2000-01-01 01:12:23.123456', TIMESTAMP '2000-01-01 01:12:23.123456'])
SELECT NOT CAST(t0.c0 AS TIME)>=('12:34:56') FROM values ('2030-01-01'::TIMESTAMP_MS), ('1969-12-23 20:44:40'::TIMESTAMP_MS) as t0(c0)
select '2021-11-15 02:30:00'::TIMESTAMP::TIMESTAMPTZ
SELECT '1880-05-15T12:00:00+00:50:20'::TIMESTAMPTZ
SELECT TIMESTAMP '2021-05-25 04:55:03.382494 UTC'
select try_cast('1111-11-11 11:11' as timestamp)
select timestamptz '2020-12-31 21:25:58.745232+0000'
SELECT 100::UHUGEINT // 0::UHUGEINT
SELECT 100::UHUGEINT % 0::UHUGEINT
SELECT 9223372036854775807::UHUGEINT::BIGINT
SELECT NULL::UHUGEINT
SELECT COUNT(*) FROM uhugeints WHERE h < '1267650600228229401496703205376'::UHUGEINT
SELECT 1080863910568919040::UHUGEINT * 1080863910568919040::UHUGEINT
SELECT union_tag(1::INTEGER::UNION(f1 VARCHAR, t DOUBLE, f2 BOOLEAN))
SELECT union_tag(1::INTEGER::UNION(f1 VARCHAR, t2 BIGINT)::UNION(F1 VARCHAR, T2 BIGINT, F3 TINYINT))
SELECT id, union_tag(a) as tag, a.b as v1, a.c as v2 FROM tbl1 UNION SELECT id, union_tag(d) as tag, d.e as v1, d.f as v2 FROM tbl2 ORDER BY ALL
SELECT tbl1.a.c, tbl1.id, tbl2.id FROM tbl2 JOIN tbl1 ON tbl1.a.c = tbl2.d.f ORDER BY ALL
SELECT union_struct.str FROM tbl1
SELECT u FROM tbl2
SELECT (20)::UTINYINT + (200)::USMALLINT
SELECT 100::UINTEGER * 100::DECIMAL(3,0)
SELECT -42::BIGINT::UBIGINT
SELECT -42::FLOAT::UTINYINT
SELECT (65534.32)::REAL::USMALLINT
SELECT [1] IS DISTINCT FROM NULL::VARIANT
SELECT []::VARIANT IS DISTINCT FROM [1, 2]
SELECT {'x': 1, 'y': ['duck', 'somateria']}::VARIANT IS NOT DISTINCT FROM NULL
SELECT {'x': 1, 'y': ['duck', 'somateria']}::VARIANT IS DISTINCT FROM {'x': 1, 'y': ['duck', 'somateria']}
SELECT sum(unique1) over (order by unique1 rows between 2 preceding and 2 following) su FROM tenk1 order by unique1
with source as ( select i, i * 5 % 11 as permuted, if(permuted < 6, NULL, permuted) as missing from range(11) tbl(i) ) select i, permuted, fill(missing order by permuted) over (partition by permuted // 5 order by i) as filled from source qualify filled is distinct from permuted order by i
SELECT c1, c3, c2, LAG(c3, c2, BITSTRING'010101010') OVER (PARTITION BY c1 ORDER BY c3) FROM issue17266 ORDER BY c1
SELECT four, ten//4 as two, sum(ten//4) OVER w st, last_value(ten//4) OVER w lt FROM tenk1d WINDOW w AS (partition by four order by ten//4 range between unbounded preceding and current row) order by four, ten//4
WITH t(r, i, p, f) AS (VALUES (0, NULL, 1, 2), (1, 1, 1, 2), (2, 2, 1, 2), (3, 3, 1, 2), (4, 4, 1, 2), (5, 5, 1, 2) ) SELECT r, QUANTILE_DISC(i, [0.25, 0.5, 0.75]) OVER (ORDER BY r ROWS BETWEEN p PRECEDING and f FOLLOWING) FROM t ORDER BY 1
SELECT r, quantile_cont(i, 0.5) OVER (ORDER BY r ROWS BETWEEN 2 PRECEDING AND 1 FOLLOWING) q FROM (VALUES (0, 0), (1, 1), (2, 2), (3, 3), (4, 0), (5, 1) ) tbl(r, i) ORDER BY 1, 2
SELECT i, LEAD(i, 1) OVER(), LEAD(i, 2) OVER() FROM range(10) tbl(i)
SELECT SUM(s) FROM ( SELECT SUM(i) OVER(ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) s FROM range(5000) tbl(i) )
WITH start_and_inputs AS ( SELECT 50 AS moves UNION ALL SELECT 50 AS moves ) SELECT moves, sum(moves) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS sum FROM start_and_inputs
select submission_date, dbsystem, tps, count(distinct dbsystem) over w AS competing, rank(order by tps desc) over w AS new_rank, first_value(tps order by tps desc) over w AS best_performance, first_value(dbsystem order by tps desc) over w AS best_system, lead(tps order by tps desc) over w AS second_performance, lead(dbsystem order by tps desc, dbsystem) over w AS second_system, from '{DATA_DIR}/csv/tpcc_results.csv' window w as ( order by submission_date range between unbounded preceding and current row ) ORDER BY ALL
select * from (select i, lag(i) over named_window from (values (1), (2), (3)) as t (i) window named_window as (order by i)) t1
SELECT part, id, sum(val) OVER(PARTITION BY part ORDER BY id), lead(val) OVER(PARTITION BY part ORDER BY id) FROM (SELECT range AS id, range % 5 AS part, range AS val FROM range(13)) t ORDER BY ALL
SELECT row_number() OVER win FROM t3 WINDOW win AS ( ORDER BY c, b, a ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING EXCLUDE CURRENT ROW )
select j, s, string_agg(s) over (partition by j order by s) from a order by j, s
