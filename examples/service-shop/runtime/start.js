import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createClient, createServer} from './http.js';

const modulePath = resolve(process.argv[2] || '/app/services/current/server.js');
const {createHandler} = await import(pathToFileURL(modulePath));
const server = createServer(createHandler({call: createClient(process.env)}));
const port = Number(process.env.PORT || 8080);
server.listen(port, process.env.HOST || '0.0.0.0', () => {
  console.log(`Demo service listening on ${server.address().port}; in-memory data and mock payments only`);
});
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close(() => process.exit(0)));
