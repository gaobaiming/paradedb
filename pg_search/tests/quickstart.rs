mod fixtures;

use approx::assert_relative_eq;
use fixtures::*;
use pgvector::Vector;
use rstest::*;
use sqlx::PgConnection;

#[rstest]
fn quickstart(mut conn: PgConnection) {
    r#"
    CALL paradedb.create_bm25_test_table(
      schema_name => 'public',
      table_name => 'mock_items'
    )
    "#
    .execute(&mut conn);

    let rows: Vec<(String, i32, String)> = r#"
    SELECT description, rating, category
    FROM mock_items
    LIMIT 3;
    "#
    .fetch(&mut conn);
    assert_eq!(
        rows,
        vec![
            ("Ergonomic metal keyboard".into(), 4, "Electronics".into()),
            ("Plastic Keyboard".into(), 4, "Electronics".into()),
            ("Sleek running shoes".into(), 5, "Footwear".into())
        ]
    );

    r#"
    CALL paradedb.create_bm25(
            index_name => 'search_idx',
            schema_name => 'public',
            table_name => 'mock_items',
            key_field => 'id',
            text_fields => '{description: {tokenizer: {type: "en_stem"}}, category: {}}'
    );
    "#
    .execute(&mut conn);

    let rows: Vec<(String, i32, String)> = r#"
    SELECT description, rating, category
    FROM search_idx.search('description:keyboard OR category:electronics', stable_sort => true, limit_rows => 5);
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].0, "Plastic Keyboard".to_string());
    assert_eq!(rows[1].0, "Ergonomic metal keyboard".to_string());
    assert_eq!(rows[2].0, "Innovative wireless earbuds".to_string());
    assert_eq!(rows[3].0, "Fast charging power bank".to_string());
    assert_eq!(rows[4].0, "Bluetooth-enabled speaker".to_string());

    let rows: Vec<(String, i32, String)> = r#"
    SELECT description, rating, category
    FROM search_idx.search('description:"bluetooth speaker"~1', stable_sort => true, limit_rows => 5);
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Bluetooth-enabled speaker");

    r#"
    CALL paradedb.create_bm25(
            index_name => 'ngrams_idx',
            schema_name => 'public',
            table_name => 'mock_items',
            key_field => 'id',
            text_fields => '{description: {tokenizer: {type: "ngram", min_gram: 4, max_gram: 4, prefix_only: false}}, category: {}}'
    );
    "#.execute(&mut conn);
    let rows: Vec<(String, i32, String)> = r#"
    SELECT description, rating, category
    FROM ngrams_idx.search('description:blue', stable_sort => true);
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Bluetooth-enabled speaker");

    let rows: Vec<(String, String, f32)> = r#"
    SELECT description, paradedb.highlight(id, field => 'description'), paradedb.rank_bm25(id)
    FROM ngrams_idx.search('description:blue', stable_sort => true)
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Bluetooth-enabled speaker");
    assert_eq!(rows[0].1, "<b>Blue</b>tooth-enabled speaker");
    assert_relative_eq!(rows[0].2, 2.9903657, epsilon = 1e-6);

    r#"
    CREATE EXTENSION vector;
    ALTER TABLE mock_items ADD COLUMN embedding vector(3);
    "#
    .execute(&mut conn);
    r#"
    UPDATE mock_items m
    SET embedding = ('[' ||
        ((m.id + 1) % 10 + 1)::integer || ',' ||
        ((m.id + 2) % 10 + 1)::integer || ',' ||
        ((m.id + 3) % 10 + 1)::integer || ']')::vector;
    "#
    .execute(&mut conn);
    let rows: Vec<(String, i32, String, Vector)> = r#"
    SELECT description, rating, category, embedding
    FROM mock_items
    LIMIT 3;
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, "Ergonomic metal keyboard");
    assert_eq!(rows[1].0, "Plastic Keyboard");
    assert_eq!(rows[2].0, "Sleek running shoes");
    assert_eq!(rows[0].3, Vector::from(vec![3.0, 4.0, 5.0]));
    assert_eq!(rows[1].3, Vector::from(vec![4.0, 5.0, 6.0]));
    assert_eq!(rows[2].3, Vector::from(vec![5.0, 6.0, 7.0]));

    r#"
    CREATE INDEX on mock_items
    USING hnsw (embedding vector_l2_ops);
    "#
    .execute(&mut conn);
    let rows: Vec<(String, String, i32, Vector)> = r#"
    SELECT description, category, rating, embedding
    FROM mock_items
    ORDER BY embedding <-> '[1,2,3]'
    LIMIT 3;
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, "Artistic ceramic vase");
    assert_eq!(rows[1].0, "Modern wall clock");
    assert_eq!(rows[2].0, "Designer wall paintings");
    assert_eq!(rows[0].3, Vector::from(vec![1.0, 2.0, 3.0]));
    assert_eq!(rows[1].3, Vector::from(vec![1.0, 2.0, 3.0]));
    assert_eq!(rows[2].3, Vector::from(vec![1.0, 2.0, 3.0]));

    let rows: Vec<(i64, f32)> = r#"
    SELECT * FROM search_idx.rank_hybrid(
        bm25_query => 'description:keyboard OR category:electronics',
        similarity_query => '''[1,2,3]'' <-> embedding',
        bm25_weight => 0.9,
        similarity_weight => 0.1
    ) LIMIT 5;
    "#
    .fetch(&mut conn);
    assert_eq!(rows[0].0, 2); // For integer comparison, regular assert_eq! is fine
    assert_eq!(rows[1].0, 1);
    assert_eq!(rows[2].0, 29);
    assert_eq!(rows[3].0, 39);
    assert_eq!(rows[4].0, 9);
    assert_relative_eq!(rows[0].1, 0.95714283, epsilon = 1e-6); // Adjust epsilon as needed
    assert_relative_eq!(rows[1].1, 0.8487012, epsilon = 1e-6);
    assert_relative_eq!(rows[2].1, 0.1, epsilon = 1e-6);
    assert_relative_eq!(rows[3].1, 0.1, epsilon = 1e-6);
    assert_relative_eq!(rows[4].1, 0.1, epsilon = 1e-6);

    let rows: Vec<(String, String, Vector, f32)> = r#"
    SELECT m.description, m.category, m.embedding, s.rank_hybrid
    FROM mock_items m
    LEFT JOIN (
        SELECT * FROM search_idx.rank_hybrid(
            bm25_query => 'description:keyboard OR category:electronics',
            similarity_query => '''[1,2,3]'' <-> embedding',
            bm25_weight => 0.9,
            similarity_weight => 0.1
        )
    ) s
    ON m.id = s.id
    LIMIT 5;
    "#
    .fetch(&mut conn);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].0, "Plastic Keyboard");
    assert_eq!(rows[1].0, "Ergonomic metal keyboard");
    assert_eq!(rows[2].0, "Designer wall paintings");
    assert_eq!(rows[3].0, "Handcrafted wooden frame");
    assert_eq!(rows[4].0, "Modern wall clock");
    assert_eq!(rows[0].2, Vector::from(vec![4.0, 5.0, 6.0]));
    assert_eq!(rows[1].2, Vector::from(vec![3.0, 4.0, 5.0]));
    assert_eq!(rows[2].2, Vector::from(vec![1.0, 2.0, 3.0]));
    assert_eq!(rows[3].2, Vector::from(vec![1.0, 2.0, 3.0]));
    assert_eq!(rows[4].2, Vector::from(vec![1.0, 2.0, 3.0]));
    assert_relative_eq!(rows[0].3, 0.95714283, epsilon = 1e-6);
    assert_relative_eq!(rows[1].3, 0.8487012, epsilon = 1e-6);
    assert_relative_eq!(rows[2].3, 0.1, epsilon = 1e-6);
    assert_relative_eq!(rows[3].3, 0.1, epsilon = 1e-6);
    assert_relative_eq!(rows[4].3, 0.1, epsilon = 1e-6);
}

#[rstest]
fn identical_queries(mut conn: PgConnection) {
    r#"
    CALL paradedb.create_bm25_test_table(
      schema_name => 'public',
      table_name => 'mock_items'
    );
    CALL paradedb.create_bm25(
            index_name => 'search_idx',
            schema_name => 'public',
            table_name => 'mock_items',
            key_field => 'id',
            text_fields => '{description: {}, category: {}}'
    );
    "#
    .execute(&mut conn);

    let rows1: SimpleProductsTableVec =
        "SELECT * FROM search_idx.search('description:shoes', stable_sort => true)"
            .fetch_collect(&mut conn);
    let rows2: SimpleProductsTableVec = "SELECT * FROM search_idx.search(
            query => paradedb.parse('description:shoes'),
            stable_sort => true
        )"
    .fetch_collect(&mut conn);
    let rows3: SimpleProductsTableVec = r#"
        SELECT * FROM search_idx.search(
	        query => paradedb.term(
	        	field => 'description',
	        	value => 'shoes'
	        ),
	        stable_sort => true
        )"#
    .fetch_collect(&mut conn);

    assert_eq!(rows1.id, rows2.id);
    assert_eq!(rows2.id, rows3.id);
}
