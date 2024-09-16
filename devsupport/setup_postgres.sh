#!/usr/bin/env bash

sqlx database create
sqlx migrate run
