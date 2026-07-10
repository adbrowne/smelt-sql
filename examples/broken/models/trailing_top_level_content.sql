-- A model file must contain at most one query body. Anything left over
-- after the model body is trailing content and must surface as a
-- TrailingTopLevelContent diagnostic instead of being silently absorbed.
SELECT id FROM users

SELECT id FROM orders
