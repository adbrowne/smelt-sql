with data as (
    select
        cast('1' as string) as string_col,
        cast(7 as integer) as numerator,
        cast(2 as integer) as denominator
)
select
    length(string_col) as length_of_string_col,
    lower(string_col) as lower_string_col,
    -- Integer / Integer infers Double (DuckDB/Spark-aligned), not truncating Integer.
    numerator / denominator as success_rate
from data
