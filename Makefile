SHELL=bash
MAIN=build

BUILD=build

.PHONY: all
all: build

.PHONY: build
build: Dockerfile
	docker build -t berlin_rs .

.PHONY: run
run: build
	docker run -ti --rm berlin_rs
