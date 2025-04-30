# Cloudflare DDNS
Auto update the cloudflare DNS for dynamic IPs.

## Download
Check the [last release](https://github.com/Vrtgs/cloudflare-ddns/releases). Download the executable that matches your system

After downloading unzip the folder.

## Configuration
There would be 4 files, edit `api.toml` with the following format:
```
[account]
email     = "mail@example.com"
api-token = "8dY3nH-As0krmv83n3pm1l"

[zone]
id     = "e3ed5fb820dd3ccc3be5f15765d329ad"
record = "subdomain.domain.tld"
# proxied = false
```

the `api-token` should be generated [from here](https://dash.cloudflare.com/profile/api-tokens) with "Edit zone DNS"

## License
TBD
