# Results fake-server TLS

These are invented, test-only TLS credentials for the loopback fake server.
The private key is public test data and must never be used outside tests.
No cluster or Results installation was contacted.

Generated with OpenSSL, using a validity of 7,300 days and loopback IP SANs:

```sh
openssl req -x509 -newkey rsa:2048 -nodes -keyout loopback-raw-key.pem \
  -out loopback-cert.pem -days 7300 -subj '/CN=Results test server' \
  -addext 'subjectAltName=IP:127.0.0.1,IP:::1' \
  -addext 'basicConstraints=critical,CA:FALSE'
openssl pkcs8 -topk8 -nocrypt -in loopback-raw-key.pem -out loopback-key.pem
```

`loopback-key.pem` holds the resulting PKCS#8 key. The tests load both files
as data; they do not require an OpenSSL executable.

`service-cert.pem` uses the same public test key and has only the DNS SAN
`results.acme.svc`, so forwarding tests cannot pass by checking the loopback
IP instead of the Service's name. Generated with:

```sh
openssl req -x509 -key loopback-key.pem -out service-cert.pem -days 7300 \
  -subj '/CN=Results test service' \
  -addext 'subjectAltName=DNS:results.acme.svc' \
  -addext 'basicConstraints=critical,CA:FALSE'
```
