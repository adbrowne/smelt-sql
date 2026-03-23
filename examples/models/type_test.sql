with data as (
    select cast('1' as string) as string_col
)
select 
    length(string_col) as length_of_string_col,
    lower(string_col) as lower_string_col