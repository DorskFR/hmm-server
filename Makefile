DATABASE_URL_TEST ?= postgres://postgres:hmm@127.0.0.1:5499/hmm_test
export DATABASE_URL_TEST

.PHONY: fmt lint test db/up db/down schema

fmt:
	cargo +nightly fmt

lint:
	cargo clippy --all-targets -- -D warnings

test:  ## Needs `make db/up`; without a database the ingest tests are skipped
	cargo test

db/up:
	docker run -d --rm --name hmm-server-test-pg --tmpfs /var/lib/postgresql \
		-e POSTGRES_PASSWORD=hmm -e POSTGRES_DB=hmm_test -p 127.0.0.1:5499:5432 postgres:18-alpine

db/down:
	docker rm -f hmm-server-test-pg

schema:  ## Refresh the envelope schema from the published client
	curl -fsSL https://unpkg.com/@dorsk/hmm/schema/envelope.v1.json -o schema/envelope.v1.json
