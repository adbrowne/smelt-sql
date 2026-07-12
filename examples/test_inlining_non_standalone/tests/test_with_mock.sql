smelt.test upstream_with_mock AS (
    SELECT * FROM smelt.upstream
)
PASSING upstream AS (
    {region: 'us-east'}
)
EXPECT (
    {region: 'us-east'}
)
