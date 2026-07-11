smelt.test upstream_no_mock AS (
    SELECT * FROM smelt.upstream
)
EXPECT (
    {region: 'us-east'}
)
