FROM golang:1.22-alpine AS builder
WORKDIR /app
COPY . .
RUN go get github.com/ClickHouse/clickhouse-go/v2@latest && go mod tidy
RUN CGO_ENABLED=0 GOOS=linux go build -ldflags="-s -w" -o server main.go

FROM alpine:3.19
WORKDIR /app
RUN apk add --no-cache ca-certificates tzdata
COPY --from=builder /app/server /app/server
EXPOSE 8791 8788
ENTRYPOINT ["/app/server"]
