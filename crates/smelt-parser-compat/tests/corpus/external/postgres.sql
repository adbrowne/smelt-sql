# Vendored from PostgreSQL REL_16_4 by scripts/extract-sql-corpus.py.
# One statement per line. Do not hand-edit; re-run the script to refresh.
# See ./README.md for license/attribution notices.

SELECT pg_advisory_unlock(1), pg_advisory_unlock(1), pg_advisory_unlock_shared(2), pg_advisory_unlock_shared(2), pg_advisory_unlock(1, 1), pg_advisory_unlock(1, 1), pg_advisory_unlock_shared(2, 2), pg_advisory_unlock_shared(2, 2)
SELECT avg(four) AS avg_1 FROM onek
SELECT var_pop(b) FROM aggtest
SELECT var_pop('inf'::float8), var_samp('inf'::float8)
select sum(null::numeric) from generate_series(1,3)
select avg(null::float8) from generate_series(1,3)
SELECT count(*), sum(x), regr_sxx(y,x), sum(y),regr_syy(y,x), regr_sxy(y,x) FROM regr_test WHERE x IN (80,100)
SELECT float8_regr_combine('{3,60,200,750,20000,2000}'::float8[], '{2,180,200,740,57800,-3400}'::float8[])
SELECT sum2(q1,q2) FROM int8_tbl
SELECT BOOL_OR(b1) AS "t", BOOL_OR(b2) AS "t", BOOL_OR(b3) AS "f", BOOL_OR(b4) AS "n", BOOL_OR(NOT b2) AS "f", BOOL_OR(NOT b3) AS "t" FROM bool_test
select max(unique2) from tenk1 order by max(unique2)
select array_agg(distinct a order by a desc nulls last) from (values (1),(2),(1),(3),(null),(2)) v(a)
select aggfns(distinct a,b,c order by b) from (values (1,3,'foo'),(0,null,null),(2,2,'bar'),(3,1,'baz')) v(a,b,c), generate_series(1,3) i
select (select count(*) from (values (1)) t0(inner_c)) from (values (2),(3)) t1(outer_c)
select my_avg(distinct one),my_sum(distinct one) from (values(1),(3),(1)) t(one)
select relname, c.oid = oldoid as orig_oid, case relfilenode when 0 then 'none' when c.oid then 'own' when oldfilenode then 'orig' else 'OTHER' end as storage, obj_description(c.oid, 'pg_class') as desc from pg_class c left join old_oids using (relname) where relname like 'at_partitioned%' order by relname
select conname, obj_description(oid, 'pg_constraint') as desc from pg_constraint where conname like 'at_partitioned%' order by conname
SELECT attinhcount, attislocal FROM pg_attribute WHERE attrelid = 'part_3_4'::regclass AND attnum > 0
SELECT f1[0:1] FROM POINT_TBL
SELECT array_cat(ARRAY[1,2], ARRAY[3,4]) AS "{1,2,3,4}"
SELECT array_position(ARRAY['sun','mon','tue','wed','thu','fri','sat'], 'mon')
SELECT array_position(ARRAY['sun','mon','tue','wed','thu',NULL,'fri','sat'], NULL)
SELECT * FROM array_op_test WHERE i && '{32}' ORDER BY seqno
SELECT * FROM array_op_test WHERE i @> '{}' ORDER BY seqno
SELECT * FROM array_op_test WHERE i = '{NULL}' ORDER BY seqno
SELECT * FROM array_op_test WHERE t @> '{AAAAAAAA72908,AAAAAAAAAA646}' ORDER BY seqno
select 33 * any (44)
select 'foo' like all (array['f%', '%o'])
select array_fill(null::integer, array[3,3],array[2,2])
select array_fill('juhu'::text, array[3,3],array[2,2])
select array_fill(1, null, array[2,2])
select array_agg(ar) from (values ('{1,2}'::int[]), ('{3}'::int[])) v(ar)
SELECT trim_array(ARRAY[]::int[], 1)
SELECT array_dims(array_sample('{{{1,2},{3,NULL}},{{5,6},{7,8}},{{9,10},{11,12}}}'::int[], 2))
SELECT b, length(b) AS lb FROM BIT_TABLE
SELECT b, SUBSTRING(b FROM 2 FOR 4) AS sub_2_4, SUBSTRING(b FROM 7 FOR 13) AS sub_7_13, SUBSTRING(b FROM 6) AS sub_6 FROM BIT_TABLE
SELECT POSITION(B'111010110' IN B'000111010110')
SELECT set_bit(B'0101011000100100', 15, 1)
SELECT * FROM pg_input_error_info('01010Z01', 'bit(8)')
SELECT pg_input_is_valid('x01010Z01', 'varbit')
SELECT bool 'on_' AS error
SELECT pg_input_is_valid('true', 'bool')
SELECT true::boolean::text AS true, false::boolean::text AS false
SELECT height(f1), width(f1) FROM BOX_TBL
SELECT * FROM box_temp WHERE f1 << '(10,20),(30,40)'
SELECT * FROM box_temp WHERE f1 &< '(10,4.333334),(5,100)'
SELECT * FROM box_temp WHERE f1 &<| '(10,4.3333334),(5,1)'
SELECT * FROM box_temp WHERE f1 <@ '(10,15),(30,35)'
SELECT brin_summarize_range('brin_summarize_multi_idx', 4294967296)
SELECT CASE WHEN i >= 3 THEN i END AS ">= 3 or Null" FROM CASE_TBL
SELECT char 'c' = char 'c' AS true
SELECT c.* FROM CHAR_TBL c WHERE c.f1 <= 'a'
SELECT '\377'::text::"char"
SELECT a,b,c,substring(d for 30), length(d) from clstr_tst
SELECT a,b,c,substring(d for 30), length(d) from clstr_tst ORDER BY b
SELECT pg_class.relname FROM pg_index, pg_class, pg_class AS pg_class_2 WHERE pg_class.oid=indexrelid AND indrelid=pg_class_2.oid AND pg_class_2.relname = 'clstr_tst' AND indisclustered
SELECT 'bbc' COLLATE "en-x-icu" > 'äbc' COLLATE "en-x-icu" AS "true"
SELECT * FROM collate_test1 WHERE b LIKE 'abc'
SELECT relname FROM pg_class WHERE relname ~* '^abc'
SELECT a, b::testdomain FROM collate_test2 ORDER BY 2
SELECT array_agg(b ORDER BY b) FROM collate_test3
SELECT a, b FROM collate_test3 WHERE a < 4 INTERSECT SELECT a, b FROM collate_test3 WHERE a > 1 ORDER BY 2
SELECT collname, nspname, obj_description(pg_collation.oid, 'pg_collation') FROM pg_collation JOIN pg_namespace ON (collnamespace = pg_namespace.oid) WHERE collname LIKE 'test%' ORDER BY 1
SELECT 'abc' <= 'ABC' COLLATE case_sensitive, 'abc' >= 'ABC' COLLATE case_sensitive
SELECT x FROM test3cs WHERE x ~ 'a'
SELECT x, count(*) FROM test3cs GROUP BY x ORDER BY x
SELECT x FROM test3ci WHERE x <> 'abc'
SELECT x FROM test2ci UNION SELECT x FROM test1ci ORDER BY x
SELECT x FROM test2ci EXCEPT SELECT x FROM test1ci
SELECT count(DISTINCT x) FROM test3ci
SELECT x FROM test3bpci WHERE x LIKE 'a%'
SELECT string_to_array('ABC,DEF,GHI'::char(11) COLLATE case_insensitive, ',', 'abc')
SELECT x FROM test4c WHERE x LIKE 'ABC%' COLLATE case_sensitive
SELECT relname FROM pg_class WHERE 'PG_CLASS'::text = relname COLLATE case_insensitive
SELECT * FROM test31_1
SELECT 'bbc' COLLATE "sv_SE" > 'äbc' COLLATE "sv_SE" AS "false"
SELECT 'bıt' ~* 'BIT' COLLATE "en_US" AS "false"
SELECT * FROM collate_test10 WHERE (x COLLATE "POSIX", y COLLATE "C") NOT IN (SELECT y, x FROM collate_test10)
SELECT collation for ('foo')
SELECT 'one' AS one, nextval('insert_seq')
select description, inbytes, (test_conv(inbytes, 'utf8', 'latin1')).* from utf8_inputs
select description, inbytes, (test_conv(inbytes, 'utf8', 'latin2')).* from utf8_inputs
select description, inbytes, (test_conv(inbytes, 'big5', 'utf8')).* from big5_inputs
select description, inbytes, (test_conv(inbytes, 'mule_internal', 'sjis')).* from mic_inputs
SELECT * FROM vistest
SELECT amname FROM pg_class c, pg_am am WHERE c.relam = am.oid AND c.oid = 'heapmv'::regclass
SELECT pg_get_functiondef('functest_S_15'::regproc)
SELECT * FROM voidtest5(3)
SELECT * FROM point_tbl WHERE f1 IS NULL
SELECT * FROM array_index_op_test WHERE i <@ '{}' ORDER BY seqno
SELECT count(*) FROM radix_text_tbl WHERE t = 'P0123456789abcdefF'
SELECT * FROM quad_point_tbl_ord_seq2 seq FULL JOIN kd_point_tbl_ord_idx2 idx ON seq.n = idx.n WHERE seq.dist IS DISTINCT FROM idx.dist
SELECT class, a FROM c_star* x WHERE x.c ~ text 'hi'
SELECT * FROM e_star*
SELECT class, aa, a FROM a_star*
SELECT 2 != 1
SELECT relname, relkind, relpersistence FROM pg_class WHERE relname ~ '^unlogged\d' ORDER BY relname
SELECT * INTO TABLE ramp FROM ONLY road WHERE name ~ '.*Ramp'
SELECT relname FROM pg_class WHERE relname LIKE 'nontemp%' AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = 'testviewschm2') ORDER BY relname
select pg_get_viewdef('v4', true)
select * from tt14v
select a from tt27v where a > 0
SELECT date 'January 8, 99 BC'
SELECT date '01-08-99'
SELECT date '1999 01 08'
SELECT date '99 08 01'
SELECT date 'yesterday' - date 'today' AS "One day"
SELECT EXTRACT(QUARTER FROM DATE '2020-08-11')
SELECT EXTRACT(DOW FROM DATE '2020-08-16')
SELECT EXTRACT(EPOCH FROM DATE '2020-08-11')
SELECT EXTRACT(DAY FROM DATE '-infinity')
SELECT EXTRACT(ISODOW FROM DATE 'infinity')
select make_date(0, 7, 15)
SELECT size, pg_size_pretty(size), pg_size_pretty(-1 * size) FROM (VALUES (10::bigint), (1000::bigint), (1000000::bigint), (1000000000::bigint), (1000000000000::bigint), (1000000000000000::bigint)) x(size)
SELECT cast('12345' as domainvarchar)
select pg_typeof(coalesce(4::domainint4, 7::domainint4))
select * from pg_input_error_info('-1', 'positiveint')
select doubledecrement(null)
select null::inotnull
SELECT ctid, oprnegate FROM pg_catalog.pg_operator fk WHERE oprnegate != 0 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_operator pk WHERE pk.oid = fk.oprnegate)
SELECT 'mauve'::rainbow
SELECT e.evtname, pg_describe_object('pg_event_trigger'::regclass, e.oid, 0) as descr, b.type, b.object_names, b.object_args, pg_identify_object(a.classid, a.objid, a.objsubid) as ident FROM pg_event_trigger as e, LATERAL pg_identify_object_as_address('pg_event_trigger'::regclass, e.oid, 0) as b, LATERAL pg_get_object_address(b.type, b.object_names, b.object_args) as a ORDER BY e.evtname
select return_int_input(1) in (null, null, null, null, null, null, null, null, null, null, null)
select return_int_input(1) not in (10, 9, 2, 8, 3, 7, 4, 6, 5, 0)
SELECT f.* FROM FLOAT4_TBL f WHERE '1004.3' > f.f1
SELECT f.f1, f.f1 * '-10' AS x FROM FLOAT4_TBL f WHERE f.f1 > '0.0'
SELECT '9223372036854775807'::float4::int8
SELECT '-10e-400'::float8
SELECT pg_input_is_valid('xyz', 'float8')
SELECT 'NaN'::float8
SELECT 'nan'::float8 / '0'::float8
SELECT power(float8 '-0.1', float8 'inf')
SELECT power(float8 '1.1', float8 'inf')
SELECT 0 ^ 0 + 0 ^ 1 + 0 ^ 0.0 + 0 ^ 0.5
SELECT exp(f.f1) from FLOAT8_TBL f
SELECT tanh(float8 'infinity')
SELECT '2147483647.4'::float8::int4
SELECT * FROM information_schema.foreign_server_options ORDER BY 1, 2, 3
SELECT * FROM fkpart9.pk
SELECT * FROM fkpart9.fk
SELECT attrelid, attname, attgenerated FROM pg_attribute WHERE attgenerated NOT IN ('', 's')
SELECT table_name, column_name, column_default, is_nullable, is_generated, generation_expression FROM information_schema.columns WHERE table_name LIKE 'gtest_' ORDER BY 1, 2
WITH foo AS (SELECT * FROM gtest1) SELECT * FROM foo
SELECT p.f1, l.s, p.f1 ## l.s FROM POINT_TBL p, LINE_TBL l
SELECT polygon(f1) FROM PATH_TBL WHERE isclosed(f1)
SELECT f1, f1::path FROM POLYGON_TBL
SELECT p1.f1, p2.f1 FROM POLYGON_TBL p1, POLYGON_TBL p2 WHERE p1.f1 <<| p2.f1
SELECT c1.f1, c2.f1 FROM CIRCLE_TBL c1, CIRCLE_TBL c2 WHERE c1.f1 << c2.f1
select gin_clean_pending_list('gin_test_idx')
select sum(c) from gstest2 group by grouping sets(grouping sets(rollup(c), grouping sets(cube(c)))) order by 1 desc
select sum(c) from gstest2 group by grouping sets(a, grouping sets(a, cube(b))) order by 1 desc
select(select (select grouping(c) from (values (1)) v2(c) GROUP BY c) from (values (1,2)) v1(a,b) group by (a,b)) from (values(6,7)) v3(e,f) GROUP BY ROLLUP(e,f)
select(select (select grouping(a,b) from (values (1)) v2(c)) from (values (1,2)) v1(a,b) group by (a,b)) from (values(6,7)) v3(e,f) GROUP BY ROLLUP((e+1),(f+1))
select v.c, (select count(*) from gstest2 group by () having v.c) from (values (false),(true)) v(c) order by v.c
select ten, grouping(ten) from onek group by grouping sets(ten) having grouping(ten) >= 0 order by 2,1
SELECT relname FROM pg_class WHERE relname = 'reset_test'
SELECT current_user = 'regress_guc_user'
select current_setting('nosuch.setting')
SELECT v as value, hashoid(v)::bit(32) as standard, hashoidextended(v, 0)::bit(32) as extended0, hashoidextended(v, 1)::bit(32) as extended1 FROM (VALUES (0), (1), (17), (42), (550273), (207112489)) x(v) WHERE hashoid(v)::bit(32) != hashoidextended(v, 0)::bit(32) OR hashoid(v)::bit(32) = hashoidextended(v, 1)::bit(32)
SELECT v as value, hashchar(v)::bit(32) as standard, hashcharextended(v, 0)::bit(32) as extended0, hashcharextended(v, 1)::bit(32) as extended1 FROM (VALUES (NULL::"char"), ('1'), ('x'), ('X'), ('p'), ('N')) x(v) WHERE hashchar(v)::bit(32) != hashcharextended(v, 0)::bit(32) OR hashchar(v)::bit(32) = hashcharextended(v, 1)::bit(32)
SELECT v as value, hash_numeric(v)::bit(32) as standard, hash_numeric_extended(v, 0)::bit(32) as extended0, hash_numeric_extended(v, 1)::bit(32) as extended1 FROM (VALUES (0), (1.149484958), (17.149484958), (42.149484958), (149484958.550273), (2071124898672)) x(v) WHERE hash_numeric(v)::bit(32) != hash_numeric_extended(v, 0)::bit(32) OR hash_numeric(v)::bit(32) = hash_numeric_extended(v, 1)::bit(32)
SELECT v as value, time_hash(v)::bit(32) as standard, time_hash_extended(v, 0)::bit(32) as extended0, time_hash_extended(v, 1)::bit(32) as extended1 FROM (VALUES (NULL::time), ('11:09:59'), ('1:09:59'), ('11:59:59'), ('7:9:59'), ('5:15:59')) x(v) WHERE time_hash(v)::bit(32) != time_hash_extended(v, 0)::bit(32) OR time_hash(v)::bit(32) = time_hash_extended(v, 1)::bit(32)
SELECT * FROM hash_f8_heap WHERE hash_f8_heap.random = '88888888'::float8
SELECT h.seqno AS i1492, h.random AS i1 FROM hash_i4_heap h WHERE h.random = 1
SELECT satisfies_hash_partition('mchash'::regclass, 4, NULL, NULL)
SELECT satisfies_hash_partition('mchash'::regclass, 3, 1, NULL::int)
SELECT timestamp with time zone '20011227 040506.789-08'
SELECT timestamp with time zone 'J2452271T040506.789+08'
SELECT time without time zone '040506.789-08'
SELECT time without time zone 'T040506.789+08'
SELECT time without time zone 'T040506'
SELECT timestamp without time zone 'Jan 1, 4713 BC' + interval '106000000 days' AS "Feb 23, 285506"
SELECT timestamp with time zone '1999-03-01' - interval '1 second' AS "Feb 28"
SELECT (time '00:00', interval '1 hour') OVERLAPS (time '01:30', interval '1 hour') AS "False"
SELECT to_timestamp('0097/Feb/16 SELECT to_timestamp('97/2/16 8:14:30', 'FMYYYY/FMMM/FMDD FMHH:FMMI:FMSS')
SELECT to_timestamp('1997 B.C. 11 16', 'YYYY B.C. MM DD')
SELECT to_timestamp('2018-11-02 12:34:56.025', 'YYYY-MM-DD HH24:MI:SS.MS')
SELECT i, to_timestamp('2018-11-02 12:34:56.1', 'YYYY-MM-DD HH24:MI:SS.FF' || i) FROM generate_series(1, 6) i
SELECT to_date('-44-02-01','YYYY-MM-DD')
SELECT to_timestamp('2015-02-11 86400', 'YYYY-MM-DD SSSSS')
SELECT seqtypid::regtype FROM pg_sequence WHERE seqrelid = 'itest3_a_seq'::regclass
select indexdef from pg_indexes where indexname like 'idxpart_idx%'
SELECT '127::1'::inet - '127::2'::inet
SELECT a FROM (VALUES ('0.0.0.0/0'::inet), ('0.0.0.0/1'::inet), ('0.0.0.0/32'::inet), ('0.0.0.1/0'::inet), ('0.0.0.1/1'::inet), ('127.126.127.127/0'::inet), ('127.127.127.127/0'::inet), ('127.128.127.127/0'::inet), ('192.168.1.0/24'::inet), ('192.168.1.0/25'::inet), ('192.168.1.1/23'::inet), ('192.168.1.1/5'::inet), ('192.168.1.1/6'::inet), ('192.168.1.1/25'::inet), ('192.168.1.2/25'::inet), ('192.168.1.1/26'::inet), ('192.168.1.2/26'::inet), ('192.168.1.2/23'::inet), ('192.168.1.255/5'::inet), ('192.168.1.255/6'::inet), ('192.168.1.3/1'::inet), ('192.168.1.3/23'::inet), ('192.168.1.4/0'::inet), ('192.168.1.5/0'::inet), ('255.0.0.0/0'::inet), ('255.1.0.0/0'::inet), ('255.2.0.0/0'::inet), ('255.255.000.000/0'::inet), ('255.255.000.000/0'::inet), ('255.255.000.000/15'::inet), ('255.255.000.000/16'::inet), ('255.255.255.254/32'::inet), ('255.255.255.000/32'::inet), ('255.255.255.001/31'::inet), ('255.255.255.002/31'::inet), ('255.255.255.003/31'::inet), ('255.255.255.003/32'::inet), ('255.255.255.001/32'::inet), ('255.255.255.255/0'::inet), ('255.255.255.255/0'::inet), ('255.255.255.255/0'::inet), ('255.255.255.255/1'::inet), ('255.255.255.255/16'::inet), ('255.255.255.255/16'::inet), ('255.255.255.255/31'::inet), ('255.255.255.255/32'::inet), ('255.255.255.253/32'::inet), ('255.255.255.252/32'::inet), ('255.3.0.0/0'::inet), ('0000:0000:0000:0000:0000:0000:0000:0000/0'::inet), ('0000:0000:0000:0000:0000:0000:0000:0000/128'::inet), ('0000:0000:0000:0000:0000:0000:0000:0001/128'::inet), ('10:23::f1/64'::inet), ('10:23::f1/65'::inet), ('10:23::ffff'::inet), ('127::1'::inet), ('127::2'::inet), ('8000:0000:0000:0000:0000:0000:0000:0000/1'::inet), ('::1:ffff:ffff:ffff:ffff/128'::inet), ('::2:ffff:ffff:ffff:ffff/128'::inet), ('::4:3:2:0/24'::inet), ('::4:3:2:1/24'::inet), ('::4:3:2:2/24'::inet), ('ffff:83e7:f118:57dc:6093:6d92:689d:58cf/70'::inet), ('ffff:84b0:4775:536e:c3ed:7116:a6d6:34f0/44'::inet), ('ffff:8566:f84:5867:47f1:7867:d2ba:8a1a/69'::inet), ('ffff:8883:f028:7d2:4d68:d510:7d6b:ac43/73'::inet), ('ffff:8ae8:7c14:65b3:196:8e4a:89ae:fb30/89'::inet), ('ffff:8dd0:646:694c:7c16:7e35:6a26:171/104'::inet), ('ffff:8eef:cbf:700:eda3:ae32:f4b4:318b/121'::inet), ('ffff:90e7:e744:664:a93:8efe:1f25:7663/122'::inet), ('ffff:9597:c69c:8b24:57a:8639:ec78:6026/111'::inet), ('ffff:9e86:79ea:f16e:df31:8e4d:7783:532e/88'::inet), ('ffff:a0c7:82d3:24de:f762:6e1f:316d:3fb2/23'::inet), ('ffff:fffa:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:fffb:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:fffc:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:fffd:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:fffe:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:fffa:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:fffb:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:fffc:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:fffd::/128'::inet), ('ffff:ffff:ffff:fffd:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:fffe::/128'::inet), ('ffff:ffff:ffff:fffe:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:ffff:4:3:2:0/24'::inet), ('ffff:ffff:ffff:ffff:4:3:2:1/24'::inet), ('ffff:ffff:ffff:ffff:4:3:2:2/24'::inet), ('ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/0'::inet), ('ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128'::inet) ) AS i(a) ORDER BY a
SELECT * FROM pg_input_error_info('1234', 'cidr')
SELECT pg_input_is_valid('1234', 'inet')
select * from inh_fk_1 order by 1
select * from patest0 join (select f1 from int4_tbl limit 1) ss on id = f1
select * from cnullparent where f1 = 2
select tableoid::regclass, a from list_parted
SELECT * FROM pg_input_error_info('50000', 'int2vector')
SELECT int2 '0o77777'
SELECT i.* FROM INT4_TBL i WHERE i.f1 <= int2 '0'
SELECT i.* FROM INT4_TBL i WHERE i.f1 <= int4 '0'
SELECT i.* FROM INT4_TBL i WHERE i.f1 >= int4 '0'
SELECT i.f1, i.f1 + int2 '2' AS x FROM INT4_TBL i
SELECT (-2147483648)::int4 * (-1)::int2
SELECT pg_input_is_valid('10000000000000000000', 'int8')
SELECT * FROM INT8_TBL WHERE q2 > 456
SELECT to_char( (q1 * -1), '9999999999999999PR'), to_char( (q2 * -1), '9999999999999999.999PR') FROM INT8_TBL
SELECT CAST(q1 AS int4) FROM int8_tbl WHERE q2 = 456
SELECT oid::int8 FROM pg_class WHERE relname = 'pg_class'
SELECT q1, q1 << 2 AS "shl", q1 >> 3 AS "shr" FROM INT8_TBL
SELECT (-9223372036854775808)::int8 / (-1)::int8
SELECT (-9223372036854775808)::int8 / (-1)::int2
SELECT int8 '0o273'
SELECT int8 '0o'
SELECT int8 '-0x8000000000000001'
SELECT INTERVAL '01:00' AS "One hour"
SELECT pg_input_is_valid('garbage', 'interval')
SELECT pg_input_is_valid('@ 30 eons ago', 'interval')
SELECT * FROM INTERVAL_TBL
SELECT * FROM INTERVAL_TBL WHERE INTERVAL_TBL.f1 >= interval '@ 1 month'
select '100000000y 10mon -1000000000d -100000h -10min -10.000001s ago'::interval
SELECT interval '1 2:03' day to minute
SELECT interval '1 -2:03' minute to second
SELECT interval '123 11' day
SELECT interval '1 2.345' day to second(2)
SELECT interval '1 2:03.4567' day to second(2)
SELECT interval '1 2:03.45678' hour to second(2)
SELECT interval ''
select interval 'P00021015T103020' AS "ISO8601 Basic Format", interval 'P0002-10-15T10:30:20' AS "ISO8601 Extended Format"
select interval 'P1Y0M3DT4H5M6S'
select interval '2562047789 hours'
select interval '9223372036854775808 microsecond'
select interval 'PT2562047789'
select interval '-1 millennium -2147483648 years'
select interval '2147483647 years 1 decade'
select interval 'P-0.1M-2147483648D'
select interval 'P2147483647D0.5W'
select interval '0.1 2562047788:0:54.775808 ago'
select interval '2562047788.1:0:54.775808 ago'
select make_interval(secs := 'NaN')
SELECT * FROM J1_TBL t1 (a, b, c)
SELECT * FROM J1_TBL NATURAL JOIN J2_TBL
SELECT * FROM J1_TBL t1 (a, b) NATURAL JOIN J2_TBL t2 (a)
select * from x left join y on (x1 = y1 and y2 is not null)
select * from (x left join y on (x1 = y1)) left join x xx(xx1,xx2) on (x1 = xx1) where (xx2 is not null)
select count(*) from (select t3.tenthous as x1, coalesce(t1.stringu1, t2.stringu1) as x2 from tenk1 t1 left join tenk1 t2 on t1.unique1 = t2.unique1 join tenk1 t3 on t1.unique2 = t3.unique2) ss, tenk1 t4, tenk1 t5 where t4.thousand = t5.unique1 and ss.x1 = t4.tenthous and ss.x2 = t5.stringu1
select t1.q2, count(t2.*) from int8_tbl t1 left join int8_tbl t2 on (t1.q2 = t2.q1) group by t1.q2 order by 1
SELECT * FROM ( SELECT 1 as key1 ) sub1 LEFT JOIN ( SELECT sub3.key3, value2, COALESCE(value2, 66) as value3 FROM ( SELECT 1 as key3 ) sub3 LEFT JOIN ( SELECT sub5.key5, COALESCE(sub6.value1, 1) as value2 FROM ( SELECT 1 as key5 ) sub5 LEFT JOIN ( SELECT 2 as key6, 42 as value1 ) sub6 ON sub5.key5 = sub6.key6 ) sub4 ON sub4.key5 = sub3.key3 ) sub2 ON sub1.key1 = sub2.key3
select a.unique1, b.unique1, c.unique1, coalesce(b.twothousand, a.twothousand) from tenk1 a left join tenk1 b on b.thousand = a.unique1 left join tenk1 c on c.unique2 = coalesce(b.twothousand, a.twothousand) where a.unique2 < 10 and coalesce(b.twothousand, a.twothousand) = 44
select 1 from int4_tbl as i4 inner join ((select 42 as n from int4_tbl x1 left join int8_tbl x2 on f1 = q1) as ss1 right join (select 1 as z) as ss2 on true) on false, lateral (select i4.f1, ss1.n from int8_tbl as i8 limit 1) as ss3
select p.* from parent p left join child c on (p.k = c.k)
SELECT * FROM b LEFT JOIN a ON (b.a_id = a.id) WHERE (a.id IS NULL OR a.id > 0)
select count(*) from tenk1 a, tenk1 b join lateral (values(a.unique1)) ss(x) on b.unique2 = ss.x
select t1.b, ss.phv from join_ut1 t1 left join lateral (select t2.a as t2a, t3.a t3a, least(t1.a, t2.a, t3.a) phv from join_pt1 t2 join join_ut1 t3 on t2.a = t3.b) ss on t1.a = ss.t2a order by t1.a
select count(*) from simple r full outer join simple s on (r.id = 0 - s.id)
SELECT hjtest_1.a a1, hjtest_2.a a2,hjtest_1.tableoid::regclass t1, hjtest_2.tableoid::regclass t2 FROM hjtest_1, hjtest_2 WHERE hjtest_1.id = (SELECT 1 WHERE hjtest_2.id = 1) AND (SELECT hjtest_1.b * 5) = (SELECT hjtest_2.c*5) AND (SELECT hjtest_1.b * 5) < 50 AND (SELECT hjtest_2.c * 5) < 55 AND hjtest_1.a <> hjtest_2.b
SELECT '"\n\"\\"'::json
SELECT '"\v"'::json
SELECT ('"'||repeat('.', 12)||'abc"')::json
SELECT '{1:"abc"}'::json
SELECT repeat('[', 10000)::json
SELECT ''::json
SELECT array_to_json(array_agg(q),false) FROM ( SELECT $$a$$ || x AS b, y AS c, ARRAY[ROW(x.*,ARRAY[1,2,3]), ROW(y.*,ARRAY[4,5,6])] AS z FROM generate_series(1,2) x, generate_series(4,5) y) q
SELECT row_to_json(q) FROM (SELECT $$a$$ || x AS b, y AS c, ARRAY[ROW(x.*,ARRAY[1,2,3]), ROW(y.*,ARRAY[4,5,6])] AS z FROM generate_series(1,2) x, generate_series(4,5) y) q
select to_json(date 'Infinity')
SELECT row_to_json(q) FROM (SELECT 'NaN'::float8 AS "float8field") q
SELECT test_json -> 'x' FROM test_json WHERE json_type = 'object'
SELECT test_json -> 2 FROM test_json WHERE json_type = 'array'
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::json -> ''
select '"foo"'::json -> 1
SELECT json_array_length('[]')
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::json #> array['a','z','b']
SELECT c FROM json_populate_record(NULL::jsrec, '{"c": "aaa"}') q
SELECT jsb FROM json_populate_record(NULL::jsrec, '{"jsb": true}') q
SELECT rec FROM json_populate_record(NULL::jsrec, '{"rec": 123}') q
SELECT reca FROM json_populate_record(NULL::jsrec, '{"reca": ["(abc,42,01.02.2003)"]}') q
select * from json_populate_recordset(row('def',99,null)::jpop,'[{"c":[100,200,300],"x":43.2},{"a":{"z":true},"b":3,"c":"2012-01-20 10:42:53"}]') q
select * from json_populate_recordset(row(0::int),'[{"a":"1","b":"2"},{"a":"3"}]') q (a text, b text)
SELECT json_build_array(VARIADIC ARRAY['a', NULL]::text[])
SELECT json_build_object('a', NULL)
SELECT json_build_object(VARIADIC '{{1,4},{2,5},{3,6}}'::int[][])
SELECT json_build_object(json '{"a":1,"b":2}', 3)
SELECT json_object_agg(name, type) FROM foo
SELECT json_object('{a,1,b,2,3,NULL,"d e f","a b c"}')
SELECT json_object('{{a,b,c},{b,c,d}}')
select json_object('{a,b,c,"d e f"}','{1,2,3,"a b c"}')
select * from json_to_record('{"out": {"key": 1}}') as x(out json)
select * from json_to_record('{"out": {"key": 1}}') as x(out jsonb)
select to_tsvector('simple', '{"a": "aaa bbb ddd ccc", "b": ["eee fff ggg"], "c": {"d": "hhh iii"}}'::json)
select json '{ "a": "\ud83d\ud83d" }' -> 'a'
SELECT jsonb '{ "a": "null \\u0000 escape" }' as not_an_escape
SELECT '"abc def"'::jsonb
SELECT '1.3e100'::jsonb
SELECT 'truf'::jsonb
select pg_input_is_valid('{"a":true}', 'jsonb')
SELECT test_json -> 9 FROM test_jsonb WHERE json_type = 'array'
SELECT jsonb_object_keys(test_json) FROM test_jsonb WHERE json_type = 'scalar'
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::jsonb -> 1
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::jsonb -> ''
select '[{"b": "c"}, {"b": "cc"}]'::jsonb -> 'z'
select '{"a": "c", "b": null}'::jsonb -> 'b'
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::jsonb ->> null::int
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::jsonb ->> 1
select '{"a": [{"b": "c"}, {"b": "cc"}]}'::jsonb ->> 'z'
SELECT '{"x":"y"}'::jsonb = '{"x":"z"}'::jsonb
SELECT jsonb_array_length('4')
SELECT jsonb_exists_all('{"a":null, "b":"qq"}', ARRAY['c','d'])
SELECT jsonb_typeof('"hello"') AS string
SELECT jsonb_build_array('a', NULL)
SELECT jsonb_build_object('a',1,'b',1.2,'c',true,'d',null,'e',json '{"x": 3, "y": [1,2,3]}')
SELECT jsonb_build_object(r,2) FROM (SELECT 1 AS a, 2 AS b) r
SELECT '{"f2":["f3",1],"f4":{"f5":99,"f6":"stringy"}}'::jsonb#>>array['f2','1']
SELECT ia1 FROM jsonb_populate_record(NULL::jsbrec, '{"ia1": 123}') q
SELECT ia1d FROM jsonb_populate_record(NULL::jsbrec, '{"ia1d": [1, "2", null, 4]}') q
SELECT jsb FROM jsonb_populate_record(NULL::jsbrec, '{"jsb": true}') q
SELECT reca FROM jsonb_populate_record(NULL::jsbrec, '{"reca": ["(abc,42,01.02.2003)"]}') q
select * from jsonb_to_record('{"a":1,"b":"foo","c":"bar"}') as x(a int, b text, d text)
select *, c is null as c_is_null from jsonb_to_recordset('[{"a":1, "b":{"c":16, "d":2}, "x":8}]'::jsonb) as t(a int, b jsonb, c text, x int)
select * from jsonb_to_record('{"out": {"key": 1}}') as x(out jsonb)
SELECT count(*) FROM testjsonb WHERE j ? 'bar'
SELECT count(*) FROM testjsonb WHERE j @@ '$.wait == "CC" && true == $.public'
SELECT count(*) FROM testjsonb WHERE j @? '$ ? (@.wait == "CC" && true == @.public)'
SELECT '{"ff":{"a":12,"b":16}}'::jsonb
SELECT '{"a":[1,2,{"c":3,"x":4}],"c":"b"}'::jsonb @> '{"a":[{"x":4},3]}'
SELECT '["a","b","c",[1,2],null]'::jsonb -> -6
select '["a", "b"]'::jsonb || '["c", "d"]'
select '[3]'::jsonb || '{}'::jsonb
select '{"a":null , "b":2, "c":3}'::jsonb - 'a'
select jsonb_set('{"n":null, "a":1, "b":[1,2], "c":{"1":2}, "d":{"1":[2,3]}}'::jsonb, '{d,1,0}', '[1,2,3]')
select '{}'::jsonb #- '{a}'
select jsonb_insert('{"a": [0,1,2]}', '{a, 1}', '{"b": "value"}')
select jsonb_insert('{"a": [0,1,2]}', '{a, 1}', '["value1", "value2"]')
select jsonb_insert('{"a": [0,1,2]}', '{a, 2}', '"new_value"', true)
select jsonb_insert('[]', '{1}', '"new_value"', true)
select jsonb_insert('{"a": {"b": "value"}}', '{a, b}', '"new_value"', true)
select ('[1, "2", null]'::jsonb)[3]
select ('{"a": ["a1", {"b1": ["aaa", "bbb", "ccc"]}], "b": "bb"}'::jsonb)['a'][1]['b1'][2]
select ('[1, "2", null]'::jsonb)[:2]
select jsonb_to_tsvector('english', '{"a": "aaa in bbb", "b": 123, "c": 456, "d": true, "f": false, "g": null}'::jsonb, '"boolean"')
select '12345.0000000000000000000000000000000000000000000005'::jsonb::int4
select jsonb '[{"a": 1}, {"a": 2}]' @? '$[0 to 1] ? (@.a > 1)'
select jsonb_path_query('[12, {"a": 13}, {"b": 14}]', 'lax $[1].a')
select jsonb_path_query('[12, {"a": 13}, {"b": 14}]', 'lax $[0 to 10].a')
select * from jsonb_path_query('[1,"1",2,"2",null]', '$[*] ? (@ == $value)', '{"value" : "1"}')
select jsonb_path_query('{"a": {"b": 1}}', 'lax $.**{2}')
select jsonb_path_query('{"a": {"b": 1}}', 'lax $.**{1 to last}.b ? (@ > 0)')
select jsonb '{"a": {"c": {"b": 1}}}' @? '$.**.b ? ( @ > 0)'
select jsonb '{"c": {"a": -1, "b":1}}' @? '$.** ? (@.a == -1)'
select jsonb '{"c": {"a": 2, "b":1}}' @? '$.** ? (@.a == 1 - - @.b)'
select jsonb_path_query('[1,2,0,3]', '$[*] ? ((2 / @ > 0) is unknown)')
select jsonb_path_query('1', '$ + "2"')
select jsonb_path_query('2', '$ > 1')
select jsonb_path_query('null', 'true.type()')
select jsonb_path_query('null', '$.double()')
select jsonb_path_query('"1.23aaa"', '$.double()')
select jsonb_path_query('"-inf"', '$.double()')
select jsonb_path_query('true', '$.floor()', silent => true)
select jsonb_path_query('[null, 1, "a\b", "a\\b", "^a\\b$"]', 'lax $[*] ? (@ like_regex "a\\b" flag "")')
select jsonb_path_query('{}', '$.datetime()')
select jsonb_path_query('"10-03-2017 12:34 +05:20"', '$.datetime("dd-mm-yyyy HH24:MI TZH:TZM")')
select jsonb_path_query('"2017-03-10 12:34:56+3:10"', '$.datetime().type()')
select jsonb_path_query('"2017-03-10 12:34:56+3:10"', '$.datetime()')
SELECT jsonb_path_query('[{"a": 1}, {"a": 2}]', '$[*]')
SELECT jsonb_path_query_first('[{"a": 1}]', 'false')
select 'strict $'::jsonpath
select '$[$[0] ? (last > 0)]'::jsonpath
select '$.datetime("datetime template")'::jsonpath
select '((($ + 1)).a + ((2)).b ? ((((@ > 1)) || (exists(@.c)))))'::jsonpath
select '$ ? (@.a < 10.1)'::jsonpath
select '$ ? (@.a < -.1e+1)'::jsonpath
select '0755'::jsonpath
select '1.e'::jsonpath
select '_1_000.5'::jsonpath
select '"the Copyright \u00a9 sign"'::jsonpath as correct_in_utf8
SELECT lo_unlink(loid) from lotest_stash_values
SELECT lo_unlink(loid) FROM lotest_stash_values
SELECT lo_from_bytea(0, 'x')
select generate_series(0,2) as s1, generate_series((random()*.1)::int,2) as s2 order by s2 desc
select * from LINE_TBL
SELECT * FROM pg_input_error_info('{1, 1}', 'line')
SELECT pg_input_is_valid('08:00:2b:01:02:03:04:ZZ', 'macaddr8')
SELECT relispopulated FROM pg_class WHERE oid = 'mvtest_tm'::regclass
SELECT * FROM mvtest_v
SELECT two, stringu1, ten, string4 INTO TABLE tmp FROM onek
SELECT p.name, name(p.hobbies), name(equipment(p.hobbies)) FROM ONLY person p
select pg_read_file('does not exist', 0, -1)
select size > 20, isdir from pg_stat_file('postmaster.pid')
select pg_ls_dir('does not exist', false, false)
select count(*) > 0 from (select pg_tablespace_databases(oid) as pts from pg_tablespace where spcname = 'pg_default') pts join pg_database db on pts.pts = db.oid
SELECT segment_number > 0 AS ok_segment_number, timeline_id FROM pg_split_walfile_name('ffffffFF00000001000000af')
SELECT relname, attname, atttypid::regtype FROM pg_class c JOIN pg_attribute a ON c.oid = attrelid WHERE c.oid < 16384 AND reltoastrelid = 0 AND relkind = 'r' AND attstorage != 'p' ORDER BY 1, 2
SELECT m + '123.45' FROM money_data
SELECT m <= '$123.00' FROM money_data
SELECT '12345678901234567'::money
SELECT '(1)'::money
SELECT '878.08'::money / 11::smallint
SELECT (-12345678901234567)::int8::money
SELECT '-92233720368547758.08'::money - '0.01'::money
select textmultirange()
select textrange('a', null)::textmultirange
SELECT * FROM nummultirange_test WHERE nmr = '{(,5)}'
SELECT * FROM nummultirange_test WHERE nmr < '{[1000.0, 1001.0]}'
SELECT nummultirange(numrange(-4,-2), numrange(1,5)) @> numrange(1,5)
SELECT '{[1,9)}' @> '{[1,5), [8,9)}'::nummultirange
SELECT '{[1,9)}' @> '{[1,5), [6,10)}'::nummultirange
SELECT 'empty'::numrange &< nummultirange(numrange(1,2))
SELECT numrange(1,6) &< nummultirange(numrange(3,4))
SELECT nummultirange(numrange(3.5,6)) &< numrange(3,4)
SELECT nummultirange(numrange(1,6)) &< nummultirange(numrange(3,4))
SELECT nummultirange(numrange(3.5,6)) &< nummultirange(numrange(3,4))
SELECT nummultirange(numrange(3,4)) &> nummultirange(numrange(3.5,6))
SELECT 'empty'::numrange -|- nummultirange(numrange(1,2))
select numrange(1,2) << nummultirange(numrange(0,4), numrange(7,8))
select nummultirange(numrange(0,2)) << numrange(3,6)
select numrange(3,6) >> nummultirange(numrange(0,2), numrange(7,8))
SELECT nummultirange() + nummultirange()
SELECT nummultirange(numrange(1,3), numrange(4,5)) - nummultirange(numrange(2,9))
SELECT nummultirange(numrange(1,2)) * nummultirange()
select count(*) from test_multirange_gist where mr = '{}'::int4multirange
select count(*) from test_multirange_gist where mr @> '{}'::int4multirange
select count(*) from test_multirange_gist where mr << int4range(100,500)
select intr_multirange(intr(1,10))
select multirangetypes_sql(nummultirange(numrange(1,10)), ARRAY[2,20])
select array[1,1] <@ arraymultirange(arrayrange(array[1,2], array[2,1]))
SELECT parse_ident(' first . " second " ." third ". " ' || repeat('x',66) || '"')::name[]
SELECT pg_catalog.set_config('search_path', ' ', false)
SELECT t1.id1, t1.result, t2.expected FROM num_result t1, num_exp_log10 t2 WHERE t1.id1 = t2.id AND t1.result != t2.expected
SELECT power('-inf'::numeric, '-inf')
SELECT MIN(val) FROM num_data
SELECT (-9223372036854775808.4)::int8
SELECT 32767.5::int2
SELECT 'Infinity'::numeric::float8
SELECT a, ceil(a), ceiling(a), floor(a), round(a) FROM ceil_floor_round
SELECT round(5.5e131071, -131073)
SELECT round(5e-16383, 16382) = 1e-16382
SELECT trunc(5e-16383, 16382) = 0
SELECT width_bucket(3.5::float8, 3.0::float8, 3.0::float8, 888)
SELECT width_bucket(0::float8, 'NaN', 4.0::float8, 888)
SELECT width_bucket(0.0::float8, 'Infinity'::float8, 5, 10)
SELECT to_char(val, 'FM9999999990999999.099999999999999') FROM num_data
SELECT to_char('100'::numeric, 'f"ool\\"999')
SELECT to_number('123456','999G999')
SELECT pg_input_is_valid('1234.567', 'numeric(8,4)')
SELECT * FROM pg_input_error_info('0x1234.567', 'numeric')
select div(12345678901234567890, 123) * 123 + 12345678901234567890 % 123
select sqrt(1.000000000000004::numeric)
select 0.5678 ^ (-85)
select (-1.0) ^ 2147483648
select ln(1.00049687395)
select log(-12.34)
select log(3.1954752e47, 9.4792021e-73)
select scale(1.12)
select scale(-1123.12471856128)
SELECT a, b, gcd(a, b), gcd(a, -b), gcd(-b, a), gcd(-b, -a) FROM (VALUES (0::numeric, 0::numeric), (0::numeric, numeric 'NaN'), (0::numeric, 46375::numeric), (433125::numeric, 46375::numeric), (43312.5::numeric, 4637.5::numeric), (4331.250::numeric, 463.75000::numeric), ('inf', '0'), ('inf', '42'), ('inf', 'inf') ) AS v(a, b)
SELECT lcm(9999 * (10::numeric)^131068 + (10::numeric^131068 - 1), 2)
WITH t(x, bc_result) AS (VALUES ('9.0e-1', .2787536009528290), ('6.0e-1', .2041199826559248), ('3.0e-1', .1139433523068368), ('9.0e-8', .000000039086501612400118), ('6.0e-8', .000000026057668132465074), ('3.0e-8', .000000013028834261665042), ('9.0e-15', .0000000000000039086503371292489), ('6.0e-15', .0000000000000026057668914195031), ('3.0e-15', .0000000000000013028834457097535), ('9.0e-22', .00000000000000000000039086503371292664), ('6.0e-22', .00000000000000000000026057668914195110), ('3.0e-22', .00000000000000000000013028834457097555), ('9.0e-29', .000000000000000000000000000039086503371292664), ('6.0e-29', .000000000000000000000000000026057668914195110), ('3.0e-29', .000000000000000000000000000013028834457097555), ('9.0e-36', .0000000000000000000000000000000000039086503371292664), ('6.0e-36', .0000000000000000000000000000000000026057668914195110), ('3.0e-36', .0000000000000000000000000000000000013028834457097555)) SELECT '1+'||x, bc_result, log(1.0+x::numeric), log(1.0+x::numeric)-bc_result AS diff FROM t
SELECT 0.a
SELECT 0.0a
SELECT 0b
SELECT 1x
SELECT .000_005
SELECT pg_get_object_address('event trigger', '{one,two}', '{}')
SELECT p1.oid, p1.proname FROM pg_proc as p1 WHERE p1.prorettype IN ('anyelement'::regtype, 'anyarray'::regtype, 'anynonarray'::regtype, 'anyenum'::regtype) AND NOT ('anyelement'::regtype = ANY (p1.proargtypes) OR 'anyarray'::regtype = ANY (p1.proargtypes) OR 'anynonarray'::regtype = ANY (p1.proargtypes) OR 'anyenum'::regtype = ANY (p1.proargtypes) OR 'anyrange'::regtype = ANY (p1.proargtypes) OR 'anymultirange'::regtype = ANY (p1.proargtypes)) ORDER BY 2
SELECT p1.oid, p1.proname FROM pg_proc as p1 WHERE proallargtypes IS NOT NULL AND proargmodes IS NOT NULL AND array_length(proallargtypes,1) <> array_length(proargmodes,1)
SELECT c.* FROM pg_cast c, pg_proc p WHERE c.castfunc = p.oid AND ((p.pronargs > 1 AND p.proargtypes[1] != 'int4'::regtype) OR (p.pronargs > 2 AND p.proargtypes[2] != 'bool'::regtype))
WITH funcdescs AS ( SELECT p.oid as p_oid, proname, o.oid as o_oid, pd.description as prodesc, 'implementation of ' || oprname || ' operator' as expecteddesc, od.description as oprdesc FROM pg_proc p JOIN pg_operator o ON oprcode = p.oid LEFT JOIN pg_description pd ON (pd.objoid = p.oid and pd.classoid = p.tableoid and pd.objsubid = 0) LEFT JOIN pg_description od ON (od.objoid = o.oid and od.classoid = o.tableoid and od.objsubid = 0) WHERE o.oid <= 9999 ) SELECT * FROM funcdescs WHERE prodesc IS DISTINCT FROM expecteddesc AND oprdesc NOT LIKE 'deprecated%' AND prodesc IS DISTINCT FROM oprdesc
SELECT oid, proname FROM pg_proc AS p WHERE prokind = 'a' AND proargdefaults IS NOT NULL
SELECT c1.oid, f1.oid FROM pg_opclass AS c1, pg_opfamily AS f1 WHERE c1.opcfamily = f1.oid AND c1.opcmethod != f1.opfmethod
SELECT a1.amprocfamily, a1.amproc, p1.prosrc FROM pg_amproc AS a1, pg_proc AS p1 WHERE a1.amproc = p1.oid AND a1.amproclefttype = a1.amprocrighttype AND p1.provolatile != 'i'
SELECT indexrelid::regclass, indrelid::regclass, attname, atttypid::regtype, opcname FROM (SELECT indexrelid, indrelid, unnest(indkey) as ikey, unnest(indclass) as iclass, unnest(indcollation) as icoll FROM pg_index) ss, pg_attribute a, pg_opclass opc WHERE a.attrelid = indrelid AND a.attnum = ikey AND opc.oid = iclass AND (NOT binary_coercible(atttypid, opcintype) OR icoll != attcollation)
SELECT c, sum(a) FROM pagg_tab WHERE c = 'x' GROUP BY c
SELECT a, sum(b), array_agg(distinct c), count(*) FROM pagg_tab_ml GROUP BY a HAVING avg(b) < 3 ORDER BY 1, 2, 3
SELECT pg_partition_root('ptif_test')
SELECT relid, parentrelid, level, isleaf FROM pg_partition_tree('ptif_test01_index') p JOIN pg_class c ON (p.relid = c.oid)
SELECT * FROM pg_partition_tree('ptif_test_matview')
SELECT pg_partition_root('ptif_li_child')
SELECT count(*) FROM prt1 t1 LEFT JOIN LATERAL (SELECT t1.b AS t1b, t2.* FROM prt2 t2) s ON t1.a = s.b WHERE s.t1b = s.b
select * from prtx1 where not exists (select 1 from prtx2 where prtx2.a=prtx1.a and (prtx2.b=prtx1.b+1 or prtx2.c=99)) and a<20 and c=91
SELECT t1.a, t1.c, t2.b, t2.c FROM prt1_adv t1 INNER JOIN prt2_adv t2 ON (t1.a = t2.b) WHERE t1.a >= 100 AND t1.a < 300 AND t1.b = 0 ORDER BY t1.a, t2.b
SELECT t1.a, t1.c, t2.a, t2.c, t3.a, t3.c FROM plt1_adv t1 LEFT JOIN plt2_adv t2 ON (t1.a = t2.a AND t1.c = t2.c) LEFT JOIN plt1_adv t3 ON (t1.a = t3.a AND t1.c = t3.c) WHERE t1.b < 10 ORDER BY t1.a
select tableoid::regclass, * from ab
select tbl1.col1, tprt.col1 from tbl1 inner join tprt on tbl1.col1 > tprt.col1 order by tbl1.col1, tprt.col1
select explain_parallel_append('select * from listp where a = (select 1)
SELECT pg_input_is_valid('16AE7F7', 'pg_lsn')
SELECT '0/16AE7F7'::pg_lsn + 'NaN'::numeric
select cache_test(3)
select * from PField_v1 where pfname = 'PF0_2' order by slotname
select namedparmcursor_test2(20, 20)
select * from compos()
select vari(variadic array[5,6,7])
select pleast(10.2,10, -20)
select foreach_test(ARRAY[(10,20),(40,69),(35,78)]::xy_tuple[])
select testoa(1,2,1)
select consumes_rw_array(a), a from returns_rw_array(1) a
SELECT p1.f1 AS point1, p2.f1 AS point2, (p1.f1 <-> p2.f1) AS distance FROM POINT_TBL p1, POINT_TBL p2 WHERE (p1.f1 <-> p2.f1) > 3 and p1.f1 << p2.f1 ORDER BY distance, p1.f1[0], p2.f1[0]
SELECT COUNT(*) FROM point_gist_tbl WHERE f1 <@ '(0.0000009,0.0000009),(0.0000009,0.0000009)'::box
select first_el_agg_f8(x::float8) over(order by x) from generate_series(1,10) x
select * from dfunc(1,2,d := 3)
select * from dfunc('Hello', 100)
select * from dfunc('Hello')
select dfunc('a'::text, 'b')
select dfunc('a'::text, 'b', flag := true)
select x, pg_typeof(x) from anyctest(11, numrange(4,7)) x
select x, pg_typeof(x) from anyctest(11, multirange(int4range(4,7))) x
select x, pg_typeof(x) from anyctest(11, multirange(numrange(4,7))) x
SELECT atest1.*,atest5.one FROM atest1, atest5
SELECT f2 FROM atestp1
SELECT pg_input_is_valid('regress_priv_user1=r/regress_priv_user2', 'aclitem')
SELECT COUNT(*) >= 0 AS ok FROM pg_shmem_allocations
SELECT r, count(*) FROM (SELECT random() r FROM generate_series(1, 1000)) ss GROUP BY r HAVING count(*) > 1
SELECT * FROM getrngfunc1(1) AS t1
SELECT * FROM getrngfunc5(1) AS t1
SELECT * FROM getrngfunc5(1) WITH ORDINALITY AS t1(a,b,c,o)
SELECT * FROM getrngfunc9(1) WITH ORDINALITY AS t1(a,b,c,o)
SELECT * FROM (VALUES (11,12),(13,15),(16,20)) v(r1,r2), rngfunc_sql(r1,r2) WITH ORDINALITY AS f(i,s,o)
SELECT * FROM (VALUES (1),(2),(3)) v(r), ROWS FROM( rngfunc_sql(10+r,13), rngfunc_mat(10+r,13) )
SELECT * FROM (VALUES (1),(2),(3)) v(r), generate_series(10+r,20-r) WITH ORDINALITY AS f(i,o)
SELECT dup(numrange(4,7))
SELECT * FROM rngfunc()
SELECT * FROM get_users()
select * from usersview
select '(a,a)'::textrange
select * from numrange_test where nr < numrange(-1000.0, -1000.0,'[]')
select range_merge(numrange(1.0, 2.0), numrange(2.0, 3.0))
select range_intersect_agg(nr) from numrange_test where false
select count(*) from test_range_gist where ir = int4range(10,20)
select count(*) from test_range_spgist where ir @> int4range(10,20)
select rangetypes_sql(int4range(1,10), ARRAY[2,20])
select 'x' ~ 'a^(^)bcd*xy(((((($a+|)+|)+|)+$|)+|)+|)^$'
select 'xyz' ~ '((.)){0}(\2){0}' as t
select regexp_match('xy', '.|...')
SELECT to_regclass('pg_class')
SELECT to_regprocedure('pg_catalog.abs(numeric)')
SELECT to_regclass('pg_catalog.pg_class')
SELECT regrole('foo.bar')
SELECT pg_input_is_valid('ng_catalog."POSIX"', 'regcollation')
SELECT * FROM joinview
SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, rolreplication, rolbypassrls, rolconnlimit, rolpassword, rolvaliduntil FROM pg_authid WHERE rolname = 'regress_test_def_superuser'
SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, rolreplication, rolbypassrls, rolconnlimit, rolpassword, rolvaliduntil FROM pg_authid WHERE rolname = 'regress_test_user_canlogin'
SELECT * FROM document
SELECT * FROM part_document_satire WHERE f_leak(dtitle) ORDER BY did
SELECT * FROM part_document ORDER by did
SELECT * FROM z1 WHERE f_leak(b)
SELECT * FROM y2 WHERE f_leak('abc')
SELECT * FROM y2 JOIN test_qual_pushdown ON (b = abc) WHERE f_leak(abc)
WITH cte1 AS MATERIALIZED (SELECT * FROM t1 WHERE f_leak(b)) SELECT * FROM cte1
SELECT refclassid::regclass, deptype FROM pg_shdepend WHERE classid = 'pg_policy'::regclass AND refobjid IN ('regress_rls_eve'::regrole, 'regress_rls_frank'::regrole)
SELECT * FROM ref_tbl
select ROW(1,2,3) < ROW(1,3,NULL) as true
select ROW('ABC','DEF') ~<=~ ROW('DEF','ABC') as true
select thousand, hundred from tenk1 where (998, 5000) < (thousand, hundred) order by thousand, hundred
select row(1, -3)::testtype1 >= row(1, -2)::testtype1
select row(1, '(1,2)')::testtype6 < row(1, '(1,3)')::testtype6
select row(1, 2)::testtype1 *<= row(1, 3)::testtype1
select row(1, -3)::testtype1 *>= row(1, -2)::testtype1
select fullname.text from fullname
select cast (row('Jim', 'Beam') as text)
SELECT (NULL::compositetable).oid
select * from rtest_t6
SELECT * FROM shoelace_candelete
select * from cchild
select pg_get_viewdef('shoe'::regclass,true) as pretty
SELECT pg_get_functiondef(0)
SELECT pg_get_partkeydef(0)
SELECT * FROM onek WHERE onek.unique1 < 10 ORDER BY onek.unique1
SELECT onek.unique1, onek.string4 FROM onek WHERE onek.unique1 > 980 ORDER BY string4 using <, unique1 using >
SELECT * FROM foo ORDER BY f1 ASC
SELECT DISTINCT p.age FROM person* p ORDER BY age using >
SELECT f1, f1 IS DISTINCT FROM NULL as "not null" FROM disttable
SELECT 1 AS one FROM test_having HAVING 1 < 2
SELECT x.b, count(*) FROM test_missing_target x, test_missing_target y WHERE x.a = y.a GROUP BY x.b ORDER BY x.b
SELECT * FROM ctas_nodata
SELECT * FROM (SELECT 1 INTO f) bar
select round(avg(aa)), sum(aa) from a_star a2
SELECT name, #thepath FROM iexit ORDER BY name COLLATE "C", 2
SELECT setval('sequence_test'::text, 32)
SELECT * FROM pg_sequence_parameters('sequence_test4'::regclass)
SELECT JSON_OBJECT('foo': NULL::json FORMAT JSON ENCODING UTF8)
SELECT JSON_OBJECT(NULL: 1)
SELECT JSON_OBJECT(1: 1, '2': NULL, '1': 1 ABSENT ON NULL WITHOUT UNIQUE RETURNING jsonb)
SELECT JSON_ARRAY(RETURNING text FORMAT JSON ENCODING UTF8)
SELECT JSON_ARRAY(SELECT i, i FROM (VALUES (1)) foo(i))
SELECT stats_reset AS bgwriter_reset_ts FROM pg_stat_bgwriter \gset SELECT stats_reset AS wal_reset_ts FROM pg_stat_wal \gset SELECT pg_stat_reset_shared('wal')
SELECT stats_reset > :'wal_reset_ts'::timestamptz FROM pg_stat_wal
SELECT pg_stat_get_snapshot_timestamp()
SELECT pg_stat_have_stats('zaphod', 0, 0)
SELECT sum(reads) AS io_sum_shared_after_reads FROM pg_stat_io WHERE context = 'normal' AND object = 'relation' \gset SELECT :io_sum_shared_after_reads > :io_sum_shared_before_reads
SELECT stxkind FROM pg_statistic_ext WHERE stxname = 'ab1_exprstat_3'
SELECT * FROM check_estimated_rows('SELECT COUNT(*) FROM ndistinct GROUP BY a, b, c')
SELECT * FROM check_estimated_rows('SELECT COUNT(*) FROM ndistinct GROUP BY (a*5), b')
SELECT * FROM check_estimated_rows('SELECT * FROM functional_dependencies WHERE a IN (1, 51) AND b IN (''1'', ''2'')')
SELECT * FROM check_estimated_rows('SELECT * FROM functional_dependencies WHERE (a * 2) IN (2, 4, 52, 54, 102, 104, 152, 154) AND upper(b) IN (''1'', ''2'', ''26'', ''27'') AND (c + 1) IN (2, 3)')
SELECT * FROM check_estimated_rows('SELECT * FROM functional_dependencies_multi WHERE a = 0 AND b = 0')
SELECT * FROM check_estimated_rows('SELECT * FROM functional_dependencies_multi WHERE c = 0 AND d = 0')
SELECT * FROM check_estimated_rows('SELECT * FROM mcv_lists WHERE 1 = a AND ''1'' = b')
SELECT * FROM check_estimated_rows('SELECT * FROM mcv_lists WHERE 1 > a AND ''1'' > b')
SELECT * FROM check_estimated_rows('SELECT * FROM mcv_lists WHERE 4 >= a AND ''0'' >= b AND 4 >= c')
SELECT * FROM check_estimated_rows('SELECT * FROM expr_stats WHERE a = 0 AND (b || c) <= ''z'' AND (c || b) >= ''0''')
SELECT * FROM tststats.priv_test_tbl WHERE a = 1 and tststats.priv_test_tbl.* > (1, 1) is not null
SELECT U&'wrong: \+0061'
SELECT E'\\x De Ad Be Ef '::bytea
SELECT TRIM(BOTH FROM ' bunch o blanks ') = 'bunch o blanks' AS "bunch o blanks"
SELECT regexp_replace('A PostgreSQL function', 'A|e|i|o|u', 'X', 1, 2)
SELECT regexp_replace('A PostgreSQL function', 'a|e|i|o|u', 'X', 1, -1, 'i')
SELECT regexp_count('ABCABCABCABC', 'Abc', 1, '')
SELECT regexp_like('abc', 'a.c', 'g')
SELECT regexp_instr('1234567890', '(123)(4(56)(78))', 1, 1, 0, 'i', 0)
SELECT regexp_instr('1234567890', '(123)(4(56)(78))', 1, 1, 0, 'i', 3)
SELECT regexp_matches('foobarbequebaz', $re$barbeque$re$)
SELECT 'abc'::bytea LIKE '_b_'::bytea AS "true"
SELECT 'i_dio' LIKE 'i$_d%o' ESCAPE '$' AS "true"
SELECT 'bear' LIKE 'b_ear' ESCAPE '_' AS "true"
SELECT 'foo' LIKE '__%' as t, 'foo' LIKE '___%' as t, 'foo' LIKE '____%' as f
SELECT replace('abcdef', 'de', '45') AS "abc45f"
select split_part('joeuser@mydatabase','',2) AS "empty string"
select split_part('joeuser@mydatabase','',-1) AS "joeuser@mydatabase"
select 'a\\bcd' as f1, 'a\\b\'cd' as f2, 'a\\b\'''cd' as f3, 'abcd\\' as f4, 'ab\\\'cd' as f5, '\\\\' as f6
SELECT lpad('hi', 5)
SELECT unistr('d\u0061t\U000000610')
SELECT unistr('wrong: \udb99\u0061')
SELECT ((SELECT 2) UNION SELECT 2)
select (select sq1) as qq1 from (select exists(select 1 from int4_tbl where f1 = q2) as sq1, 42 as dummy from int8_tbl) sq0 join int4_tbl i4 on dummy = i4.f1
SELECT id FROM test_tablesample TABLESAMPLE SYSTEM (100.0/11) REPEATABLE (0)
select pct, count(unique1) from (values (0),(100)) v(pct), lateral (select * from tenk1 tablesample bernoulli (pct)) ss group by pct
SELECT id FROM test_tablesample TABLESAMPLE FOOBAR (1)
SELECT relname, spcname FROM pg_catalog.pg_tablespace t, pg_catalog.pg_class c where c.reltablespace = t.oid AND c.relname = 'asexecute'
select * from whereami
select 'four: '::text || 2+2
select concat(variadic array[1,2,3])
select format('>>%-10s<<', '')
SELECT currtid2('tid_seq'::text, '(0,1)'::tid)
SELECT ctid FROM tidrangescan WHERE '(2,8)' < ctid
SELECT * FROM tidscan
SELECT pg_input_is_valid('15:36:39 America/New_York', 'time')
SELECT count(*) AS two FROM TIMESTAMP_TBL WHERE d1 = timestamp(2) without time zone 'now'
SELECT date_bin('-2 days'::interval, timestamp '1970-01-01 01:00:00' , timestamp '1970-01-01 00:00:00')
SELECT to_char(d1, 'HH24 FROM TIMESTAMP_TBL
SELECT to_char(d1, 'YYYYTH YYYYth Jth') FROM TIMESTAMP_TBL
select * from generate_series('2020-01-01 00:00'::timestamp, '2020-01-02 03:00'::timestamp, '0 hour'::interval)
SELECT '20500710 173201 Europe/Helsinki'::timestamptz
SELECT d1 FROM TIMESTAMPTZ_TBL WHERE d1 > timestamp with time zone '1997-01-02'
SELECT date_trunc('day', timestamp with time zone '2001-02-16 20:38:40+00', 'GMT') as gmt_trunc
SELECT extract(epoch from '294270-01-01 00:00:00+00'::timestamptz)
SELECT to_char(d1, 'HH HH12 HH24 MI SS SSSS') FROM TIMESTAMPTZ_TBL
SELECT to_char(now(), 'of') as "Of", to_char(now(), 'tzh:tzm') as "tzh:tzm"
SELECT make_timestamptz(2008, 12, 10, 10, 10, 10, 'EST')
SELECT date_subtract('2021-10-31 00:00:00+02'::timestamptz, '1 day'::interval, 'Europe/Warsaw')
SELECT '2014-10-26 00:00:00 Europe/Moscow'::timestamptz
SELECT '2014-10-26 01:00:00 Europe/Moscow'::timestamptz
SELECT to_timestamp('-Infinity'::float)
SELECT '2014-10-25 22:00:01 UTC'::timestamptz
SELECT f1 AS "None" FROM TIMETZ_TBL WHERE f1 < '00:00-07'
SELECT date_part('epoch', TIME WITH TIME ZONE '2020-05-26 13:30:25.575401-04')
select set_ttdummy(1)
select tgrelid::regclass, tgname, tgenabled from pg_trigger where tgrelid in ('parent'::regclass, 'child1'::regclass) order by tgrelid::regclass::text, tgname
SELECT * FROM truncate_a
SELECT * FROM tp_chk_data()
SELECT ts_lexize('ispell', 'rebook')
SELECT ts_lexize('hunspell', 'booking')
SELECT ts_lexize('hunspell_long', 'unbooking')
SELECT ts_lexize('hunspell_num', 'ballyklubber')
SELECT ts_lexize('synonym', 'PoStGrEs')
SELECT to_tsvector('synonym_tst', 'Postgresql is often called as postgres or pgsql and pronounced as postgre')
SELECT to_tsvector('thesaurus_tst', 'one postgres one two one two three one')
SELECT count(*) FROM test_tsvector WHERE a @@ '!pl <-> yh'
SELECT count(*) FROM test_tsvector WHERE a @@ '!wd:D'
SELECT to_tsquery('simple', 'qwe & sKies ')
SELECT plainto_tsquery('english', 'the and z 1))& fghj')
SELECT to_tsquery('english', '(1 <-> a) <-> 2')
SELECT to_tsquery('english', '(1 <-> a) <3> 2')
SELECT to_tsquery('english', '(2 <-> (a <-> 1)) <-> s')
SELECT COUNT(*) FROM test_tsquery WHERE keyword < 'new <-> york'
SELECT COUNT(*) FROM test_tsquery WHERE keyword <= 'new <-> york'
SELECT ts_rewrite( 'moscow & hotel', 'SELECT keyword, sample FROM test_tsquery')
SELECT ts_rewrite('5 <-> (6 | 8)', 'SELECT keyword, sample FROM test_tsquery'::text )
SELECT count(*) FROM test_tsvector WHERE a @@ to_tsquery('345&qwerty')
select * from pendtest where 'ipsa:*'::tsquery @@ ts
select websearch_to_tsquery('simple', 'abc : def')
select websearch_to_tsquery('simple', 'abc:d')
select websearch_to_tsquery('simple', 'abc (pg or class)')
select websearch_to_tsquery('simple', 'cat "OR" rat')
select websearch_to_tsquery('simple', 'abc or-abc')
select websearch_to_tsquery('english', 'this is select websearch_to_tsquery('english', '(()) )))) this ||| is && -fine, "dear friend" OR good')
select websearch_to_tsquery('\')
SELECT dataa, datab b, generate_series(1,2) g, count(*) FROM few GROUP BY CUBE(dataa, datab)
SELECT * FROM fewmore
SELECT '1'::tsvector
SELECT $$'\\as' ab\c ab\\c AB\\\c ab\\\\c$$::tsvector
SELECT '!(1&2)'::tsquery
SELECT '!1|2&3'::tsquery
SELECT 'a:* & nbb:*ac | doo:a* | goo'::tsquery
SELECT 'a & !!b'::tsquery
SELECT 'a & g' <-> 'b & d'::tsquery
SELECT 'a b:89 ca:23A,64b d:34c'::tsvector @@ 'd:AC & ca:B' as "true"
SELECT 'supeznova supernova'::tsvector @@ 'super:*'::tsquery AS "true"
select to_tsvector('simple', 'q y') @@ 'q <-> (x | y <-> z)' AS "false"
SELECT ts_rank(' a:1 s:2B d g'::tsvector, 'a | s')
SELECT ts_rank_cd(' a:1 s:2B d g'::tsvector, 'a | s')
SELECT ts_rank_cd(' a:1 s:2B d g'::tsvector, 'a & s')
SELECT ts_rank_cd(' a:1 sa:2D sb:2A g'::tsvector, 'a <-> s:*')
SELECT ts_delete('base hidden rebel spaceship strike'::tsvector, ARRAY['spaceship','leya','rebel','rebel'])
SELECT ts_filter('base hidden rebel spaceship strike'::tsvector, '{a}')
SELECT t1.oid, t1.typname FROM pg_type as t1 WHERE t1.typtype = 'r' AND NOT EXISTS(SELECT 1 FROM pg_range r WHERE rngtypid = t1.oid)
SELECT t1.oid, t1.typname, p1.oid, p1.proname FROM pg_type AS t1, pg_proc AS p1 WHERE t1.typmodout = p1.oid AND NOT (p1.pronargs = 1 AND p1.proargtypes[0] = 'int4'::regtype AND p1.prorettype = 'cstring'::regtype AND NOT p1.proretset)
SELECT t1.oid, t1.typname, t1.typelem, t1.typlen, t1.typbyval FROM pg_type AS t1 WHERE t1.typsubscript = 'array_subscript_handler'::regproc AND NOT (t1.typelem != 0 AND t1.typlen = -1 AND NOT t1.typbyval)
SELECT c1.oid, c1.relname FROM pg_class as c1 WHERE c1.relkind IN ('S', 'v', 'f', 'c') and c1.relam != 0
SELECT a1.attrelid, a1.attname FROM pg_attribute as a1 WHERE a1.attrelid = 0 OR a1.atttypid = 0 OR a1.attnum = 0 OR a1.attcacheoff != -1 OR a1.attinhcount < 0 OR (a1.attinhcount = 0 AND NOT a1.attislocal)
SELECT * FROM persons
SELECT is_normalized('abc', 'def')
SELECT q1 FROM int8_tbl UNION ALL SELECT q2 FROM int8_tbl EXCEPT SELECT q1 FROM int8_tbl ORDER BY 1
SELECT q1 FROM int8_tbl UNION ALL (((SELECT q2 FROM int8_tbl EXCEPT SELECT q1 FROM int8_tbl ORDER BY 1)))
select from generate_series(1,5) union all select from generate_series(1,3)
SELECT table_name, is_insertable_into FROM information_schema.tables WHERE table_name LIKE 'rw_view%' ORDER BY table_name
SELECT * FROM v1 WHERE a=8
select * from base_tab order by a
SELECT COUNT(*) FROM guid1 WHERE guid_field < '22222222-2222-2222-2222-222222222222'
SELECT c.* FROM VARCHAR_TBL c WHERE c.f1 <= 'a'
SELECT row_number() OVER (ORDER BY unique2) FROM tenk1 WHERE unique2 < 10
SELECT sum(unique1) over (order by four range between 2::int8 preceding and 1::int2 preceding exclude ties), unique1, four FROM tenk1 WHERE unique1 < 10
select sum(salary) over (order by enroll_date range between '1 year'::interval preceding and '1 year'::interval following exclude ties), salary, enroll_date from empsalary
select id, f_numeric, first_value(id) over w, last_value(id) over w from numerics window w as (order by f_numeric range between 'inf' preceding and 'inf' following)
select id, f_numeric, first_value(id) over w, last_value(id) over w from numerics window w as (order by f_numeric range between 1.1 preceding and 'NaN' following)
SELECT sum(unique1) over (order by four groups between 1 following and unbounded following), unique1, four FROM tenk1 WHERE unique1 < 10
SELECT sum(unique1) over (order by four groups between 2 preceding and 1 following exclude current row), unique1, four FROM tenk1 WHERE unique1 < 10
select first_value(salary) over(order by enroll_date groups between 1 preceding and 1 following), lead(salary) over(order by enroll_date groups between 1 preceding and 1 following), nth_value(salary, 1) over(order by enroll_date groups between 1 preceding and 1 following), salary, enroll_date from empsalary
select last_value(salary) over(order by enroll_date groups between 1 preceding and 1 following), lag(salary) over(order by enroll_date groups between 1 preceding and 1 following), salary, enroll_date from empsalary
select f1, sum(f1) over (partition by f1 groups between 1 preceding and 1 following) from t1 where f1 = f2
select f1, sum(f1) over (partition by f1, f2 order by f2 groups between 1 following and 2 following) from t1 where f1 = f2
SELECT * FROM empsalary INNER JOIN tenk1 ON row_number() OVER (ORDER BY salary) < 10
SELECT rank() OVER (ORDER BY 1), count(*) FROM empsalary GROUP BY 1
SELECT * FROM (SELECT empno, salary, count(1) OVER (ORDER BY salary DESC) c FROM empsalary) emp WHERE c <= 3
SELECT i,SUM(v::interval) OVER (ORDER BY i ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) FROM (VALUES(1,'1 sec'),(2,'2 sec'),(3,NULL),(4,NULL)) t(i,v)
SELECT STDDEV_SAMP(n::bigint) OVER (ORDER BY i ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) FROM (VALUES(1,NULL),(2,600),(3,470),(4,170),(5,430),(6,300)) r(i,n)
SELECT i, b, bool_and(b) OVER w, bool_or(b) OVER w FROM (VALUES (1,true), (2,true), (3,false), (4,false), (5,true)) v(i,b) WINDOW w AS (ORDER BY i ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)
WITH RECURSIVE x (id) AS (SELECT 1 UNION ALL SELECT id+1 FROM y WHERE id < 5), y (id) AS (SELECT 1 UNION ALL SELECT id+1 FROM x WHERE id < 5) SELECT * FROM x
WITH RECURSIVE foo(i) AS (SELECT i FROM (VALUES(1),(2)) t(i) UNION ALL SELECT (i+1)::numeric(10,0) FROM foo WHERE i < 10) SELECT * FROM foo
WITH RECURSIVE outermost(x) AS ( SELECT 1 UNION (WITH innermost as (SELECT 2) SELECT * FROM outermost UNION SELECT * FROM innermost) ) SELECT * FROM outermost ORDER BY 1
WITH RECURSIVE outermost(x) AS ( WITH innermost as (SELECT 2 FROM outermost) SELECT * FROM innermost UNION SELECT * from outermost ) SELECT * FROM outermost ORDER BY 1
SELECT * FROM withz ORDER BY k
SELECT * FROM yy
select 'asdf'::xid8
select '12:13:'::pg_snapshot
SELECT xmlconcat('<foo/>', NULL, '<?xml version="1.1" standalone="no"?><bar/>')
SELECT xmlelement(name employee, xmlforest(name, age, salary as pay)) FROM emp
SELECT xmlelement(name foo, xmlattributes('infinity'::timestamp as bar))
SELECT xmlparse(content ' ')
SELECT xmlparse(document ' ')
SELECT xmlparse(document 'abc')
SELECT xmlpi(name foo)
SELECT xpath('/value', data) FROM xmltest
SELECT xpath('''<<invalid>>''', '<root/>')
SELECT xpath('/nosuchtag', '<root/>')
SELECT xpath_exists('count(/nosuchtag)', '<root/>'::xml)
SELECT COUNT(id) FROM xmltest WHERE xpath_exists('/menu/beers/name[text() = ''Molson'']',data)
SELECT xml_is_well_formed('<relativens xmlns=''relative''/>')
SELECT * FROM xmltable('/x/a' PASSING '<x><a><ent>&apos
SELECT schema_to_xmlschema('testxmlschema', false, true, '')
