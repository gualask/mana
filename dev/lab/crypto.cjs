// Local fixture generator: only Node built-ins, no downloaded scripts or keys.
const crypto = require('node:crypto');
const fs = require('node:fs');
const request = JSON.parse(fs.readFileSync(0, 'utf8'));
const encode = value => value.toString('base64url');
if (request.action === 'seal') {
  const key = crypto.randomBytes(32), mask = crypto.randomBytes(32);
  const iv = crypto.randomBytes(12), aad = Buffer.from('stream-synthetic-envelope:v1');
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  cipher.setAAD(aad);
  const data = Buffer.concat([cipher.update(JSON.stringify({u: request.url, e: request.expires * 1000})), cipher.final(), cipher.getAuthTag()]);
  const values = [Buffer.from(key.map((value, index) => value ^ mask[index])), mask, iv, data, aad,
    ...['crypto', 'subtle', 'importKey', 'decrypt', 'AES-GCM', 'raw'].map(value => Buffer.from(value))];
  process.stdout.write(JSON.stringify(values.map(encode)));
} else if (request.action === 'ctr') {
  const cipher = crypto.createCipheriv('aes-256-ctr', Buffer.from(request.key, 'hex'), Buffer.from(request.iv, 'hex'));
  process.stdout.write(JSON.stringify(encode(Buffer.concat([cipher.update(request.session), cipher.final()]))));
} else {
  throw new Error('Unknown local fixture action');
}
