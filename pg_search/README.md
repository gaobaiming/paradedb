<h1 align="center">
  <img src="../docs/logo/pg_search.svg" alt="pg_search" width="500px"></a>
<br>
</h1>

[![Test pg_search](https://github.com/paradedb/paradedb/actions/workflows/test-pg_search.yml/badge.svg)](https://github.com/paradedb/paradedb/actions/workflows/test-pg_search.yml)

## Overview

`pg_search` is a PostgreSQL extension that enables full text search over SQL tables using the BM25 algorithm, the state-of-the-art ranking function for full text search. It is built on top of Tantivy, the Rust-based alternative to Apache Lucene, using `pgrx`.

`pg_search` is supported on all versions supported by the PostgreSQL Global Development Group, which includes PostgreSQL 12+.

Check out the `pg_search` benchmarks [here](../benchmarks/README.md).

### Roadmap

- [x] BM25 scoring
- [x] Highlighting
- [x] Filtering
- [x] Autocomplete
- [x] Boosted queries
- [x] Fuzzy search
- [x] Custom tokenizers and multi-language support
- [x] JSON field search
- [x] Hybrid search
- [ ] Datetime fields
- [ ] Faceted search
- [ ] Generative search
- [ ] Multimodal search
- [ ] Distributed search

## Installation

### From ParadeDB

The easiest way to use the extension is to run the ParadeDB Dockerfile:

```bash
docker run \
  --name paradedb \
  -e POSTGRESQL_USERNAME=<user> \
  -e POSTGRESQL_PASSWORD=<password> \
  -e POSTGRESQL_DATABASE=<dbname> \
  -e POSTGRESQL_POSTGRES_PASSWORD=<superuser_password> \
  -v paradedb_data:/bitnami/postgresql \
  -p 5432:5432 \
  -d \
  paradedb/paradedb:latest
```

This will spin up a Postgres instance with `pg_search` preinstalled.

### From Self-Hosted PostgreSQL

If you are self-hosting Postgres and would like to use the extension within your existing Postgres, follow the steps below.

It's **very important** to make the following change to your `postgresql.conf` configuration file. `pg_search` must be in the list of `shared_preload_libraries`:

```c
shared_preload_libraries = 'pg_search'
```

This enables the extension to spawn a background worker process that performs writes to the index. If this background process is not started because of an incorrect `postgresql.conf` configuration, your database connection will crash or hang when you attempt to create a `pg_search` index.

#### Debian/Ubuntu

We provide pre-built binaries for Debian-based Linux for PostgreSQL 16. You can download the latest version for your architecture from the [releases page](https://github.com/paradedb/paradedb/releases).

Our pre-built binaries come with the ICU tokenizer enable, which requires the `libicu70` library. If you don't have it installed, you can do so with `sudo apt-get install libicu70 -y`, or compile the extension from source without `--features icu` to build without the ICU tokenizer.

ParadeDB collects anonymous telemetry to help us understand how many people are using the project. You can opt-out of telemetry by setting `export TELEMETRY=false` (or unsetting the variable) in your shell or in your `~/.bashrc` file before running the extension.

#### macOS

We don't suggest running production workloads on macOS. As a result, we don't provide prebuilt binaries for macOS. If you are running Postgres on macOS and want to install `pg_search`, please follow the [development](#development) instructions, but do `cargo pgrx install --release` instead of `cargo pgrx run`. This will build the extension from source and install it in your Postgres instance.

You can then create the extension in your database by running:

```sql
CREATE EXTENSION pg_search;
```

Note: If you are using a managed Postgres service like Amazon RDS, you will not be able to install `pg_search` until the Postgres service explicitly supports it.

#### Windows

Windows is not supported. This restriction is [inherited from pgrx not supporting Windows](https://github.com/pgcentralfoundation/pgrx?tab=readme-ov-file#caveats--known-issues).

## Usage

### Indexing

`pg_search` comes with a helper function that creates a test table that you can use for quick experimentation.

```sql
CALL paradedb.create_bm25_test_table(
  schema_name => 'public',
  table_name => 'mock_items'
);
```

To index the table, use the following SQL command:

```sql
CALL paradedb.create_bm25(
        index_name => 'search_idx',
        schema_name => 'public',
        table_name => 'mock_items',
        key_field => 'id',
        text_fields => '{description: {tokenizer: {type: "en_stem"}}, category: {}}',
        numeric_fields => '{rating: {}}'
);
```

Note the mandatory `key_field` option in the `WITH` code. Every `bm25` index needs a `key_field`, which should be the name of a column that will function as a row's unique identifier within the index. Usually, the `key_field` can just be the name of your table's primary key column.

Once the indexing is complete, you can run various search functions on it.

### Basic Search

Execute a search query on your indexed table:

```sql
SELECT description, rating, category
FROM search_idx.search(
  '(description:keyboard OR category:electronics) AND rating:>2',
  limit_rows => 5
);
```

This will return:

```csv
         description         | rating |  category
-----------------------------+--------+-------------
 Plastic Keyboard            |      4 | Electronics
 Ergonomic metal keyboard    |      4 | Electronics
 Innovative wireless earbuds |      5 | Electronics
 Fast charging power bank    |      4 | Electronics
 Bluetooth-enabled speaker   |      3 | Electronics
(5 rows)
```

Note the usage of `limit_rows` instead of the SQL `LIMIT` clause. For optimal performance, we recommend always using
`limit_rows` and `offset_rows` instead of `LIMIT` and `OFFSET`.

Similarly, the `rating:>2` filter was used instead of the SQL `WHERE` clause for [efficient filtering](https://docs.paradedb.com/search/full-text/bm25#efficient-filtering).

Advanced features like BM25 scoring, highlighting, custom tokenizers, fuzzy search, and more are supported. Please refer to the [documentation](https://docs.paradedb.com) and [quickstart](https://docs.paradedb.com/search/quickstart) for a more thorough overview of `pg_search`'s query support.

## Development

### Prerequisites

To develop the extension, first install Rust v1.76.0 using `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install 1.76.0

# We recommend setting the default version to 1.76.0 for consistency across your system
rustup default 1.76.0
```

Note: While it is possible to install Rust via your package manager, we recommend using `rustup` as we've observed inconcistencies with Homebrew's Rust installation on macOS.

Then, install the PostgreSQL version of your choice using your system package manager. Here we provide the commands for the default PostgreSQL version used by this project:

```bash
# macOS
brew install postgresql@16

# Ubuntu
wget --quiet -O - https://www.postgresql.org/media/keys/ACCC4CF8.asc | sudo apt-key add -
sudo sh -c 'echo "deb http://apt.postgresql.org/pub/repos/apt/ $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list'
sudo apt-get update && sudo apt-get install -y postgresql-16 postgresql-server-dev-16
```

If you are using Postgres.app to manage your macOS PostgreSQL, you'll need to add the `pg_config` binary to your path before continuing:

```bash
export PATH="$PATH:/Applications/Postgres.app/Contents/Versions/latest/bin"
```

Then, install and initialize `pgrx`:

```bash
# Note: Replace --pg16 with your version of Postgres, if different (i.e. --pg15, --pg14, etc.)
cargo install --locked cargo-pgrx --version 0.12.0-alpha.1

# macOS arm64
cargo pgrx init --pg16=/opt/homebrew/opt/postgresql@16/bin/pg_config

# macOS amd64
cargo pgrx init --pg16=/usr/local/opt/postgresql@16/bin/pg_config

# Ubuntu
cargo pgrx init --pg16=/usr/lib/postgresql/16/bin/pg_config
```

If you prefer to use a different version of Postgres, update the `--pg` flag accordingly.

Note: While it is possible to develop using pgrx's own Postgres installation(s), via `cargo pgrx init` without specifying a `pg_config` path, we recommend using your system package manager's Postgres as we've observed inconsistent behaviours when using pgrx's.

#### ICU Tokenizer

`pg_search` comes with multiple tokenizers for different languages. The ICU tokenizer, which enables tokenization for Arabic, Amharic, and Greek, is not enabled by default in development due to the additional dependencies it requires. To develop with the ICU tokenizer enabled, first:

Ensure that the `icu4c` library is installed. It should come preinstalled on most distros, but you can install it with your system package manager if it isn't:

```bash
# macOS
brew install icu4c

# Ubuntu
sudo apt-get install libicu70 -y
```

Additionally, on macOS you'll need to add the `icu-config` binary to your path before continuing:

```bash
# ARM macOS
export PKG_CONFIG_PATH="/opt/homebrew/opt/icu4c/lib/pkgconfig"

# Intel macOS
export PKG_CONFIG_PATH="/usr/local/opt/icu4c/lib/pkgconfig"
```

Finally, to enable the ICU tokenizer in development, pass `--features icu` to the `cargo pgrx run` and `cargo pgrx test` commands.

### Running the Extension

First, start pgrx:

```bash
cargo pgrx run
```

This will launch an interactive connection to Postgres. Inside Postgres, create the extension by running:

```sql
CREATE EXTENSION pg_search;
```

Now, you have access to all the extension functions.

### Modifying the Extension

If you make changes to the extension code, follow these steps to update it:

1. Recompile the extension:

```bash
cargo pgrx run
```

2. Recreate the extension to load the latest changes:

```sql
DROP EXTENSION pg_search;
CREATE EXTENSION pg_search;
```

### Testing

To run the unit test suite, use the following command:

```bash
cargo pgrx test
```

This will run all unit tests defined in `/src`. To add a new unit test, simply add tests inline in the relevant files, using the `#[cfg(test)]` attribute.

To run the integration test suite, simply run:

```bash
./test/runtests.sh -p sequential
```

This will create a temporary database, initialize it with the SQL commands defined in `fixtures.sql`, and run the tests in `/test/sql` against it. To add a new test, simply add a new `.sql` file to `/test/sql` and a corresponding `.out` file to `/test/expected` for the expected output, and it will automatically get picked up by the test suite.

Note: the bash script takes arguments and allows you to run tests either sequentially or in parallel. For more info run `./test/runtests.sh -h`

## License

`pg_search` is licensed under the [GNU Affero General Public License v3.0](../LICENSE).
