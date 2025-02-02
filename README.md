# cloud_kms

## 编译docker镜像
```
docker build -t citacloud/cloud_kms .
```
## 使用方法

```
$ cloud_kms -h
cloud_kms 0.1.0
Rivtower Technologies <contact@rivtower.com>

Usage: cloud_kms <COMMAND>

Commands:
  run   run this service
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### crypto-run

运行`crypto`服务。

```
$ crypto run -h
run this service

Usage: cloud_kms run [OPTIONS]

Options:
  -c, --config <CONFIG_PATH>  Chain config path [default: config.toml]
  -h, --help                  Print help
```

参数：
1. 微服务配置文件。

    参见示例`config/config.toml`。

    其中`[cloud_kms]`段为微服务的配置：
    * `port` http port
    * `master_key` master key

    其中`[cloud_kms.log_config]`段为微服务日志的配置：
    * `max_level` 日志等级
    * `filter` 日志过滤配置
    * `service_name` 服务名称，用作日志文件名与日志采集的服务名称
    * `rolling_file_path` 日志文件路径
    * `agent_endpoint` jaeger 采集端地址

```
$ crypto run -c config/config.toml
2023-09-01T16:45:46.499179+08:00  INFO cloud_kms: kms listening on 127.0.0.1:3000
```

## 服务接口

/api/v1/keys

```
$ curl --request POST \
  --url http://127.0.0.1:3000/api/v1/keys \
  --header 'Content-Type: application/json' \
  --data '{
	"user_code": "$user_code",
	"crypto_type": "Secp256k1"
}'
```

返回：

```json
{
	"code": 200,
	"data": {
		"address": "0x3ae29bc9d878bbc0d83b831a59b330f0154a596c",
		"crypto_type": "Secp256k1",
		"public_key": "028DC3BB1749AC3E1B51B9398E85FF7A9F77DD89A63A692ED4B053644D4F8AB5BC",
		"user_code": "$user_code"
	},
	"message": "OK"
}
```

/api/v1/keys/addr

```
$ curl --request POST \
  --url http://127.0.0.1:3000/api/v1/keys/addr \
  --header 'Content-Type: application/json' \
  --data '{
	"address": "6F142508B4EEA641E33CB2A0161221105086A84584C74245CA463A49EFFEA30B",
	"crypto_type": "Secp256k1"
}'
```

返回：

```json
{
	"code": 200,
	"data": {
		"address": "0xad52a9f149b1b87eb5ca4268842d463696a7f459",
		"crypto_type": "Secp256k1",
		"public_key": "02C164157C14E4B2BD63A34A0B9C83300D8E1B9A11E6D2E32C4CEC2FFE5DFEEAD2",
		"user_code": ""
	},
	"message": "OK"
}
```

/api/v1/keys/sign

```
$ curl --request POST \
  --url http://127.0.0.1:3000/api/v1/keys/sign \
  --header 'Content-Type: application/json' \
  --data '{
	"user_code": "$user_code",
	"crypto_type": "Secp256k1",
	"message": "$message"
}'
```

返回：

```json
{
	"code": 200,
	"data": {
		"signature": {
			"r": "0x14ca0f7557ae9641ef4caad2a0a0afa388ff135112ecc85095f1b668e0d76d94",
			"s": "0x1e41191ac872537cc61d9e59dd933aa2759d1523d09fb1446248166f82a6fc25",
			"v": 27
		}
	},
	"message": "OK"
}
```

/api/v1/keys/verify

```
$ curl --request POST \
  --url http://127.0.0.1:3000/api/v1/keys/verify \
  --header 'Content-Type: application/json' \
  --data '{
	"user_code": "$user_code",
	"crypto_type": "Secp256k1",
	"message": "$message",
	"signature": {
			"r": "0x14ca0f7557ae9641ef4caad2a0a0afa388ff135112ecc85095f1b668e0d76d94",
			"s": "0x1e41191ac872537cc61d9e59dd933aa2759d1523d09fb1446248166f82a6fc25",
			"v": 27
		}
}'
```

返回：

```json
{
	"code": 200,
	"data": true,
	"message": "OK"
}
```
