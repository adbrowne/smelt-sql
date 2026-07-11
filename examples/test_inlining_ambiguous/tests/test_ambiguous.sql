smelt.test ambiguous_users AS (
    SELECT * FROM smelt.users
)
EXPECT (
    {user_id: 1, name: 'alice'}
)
