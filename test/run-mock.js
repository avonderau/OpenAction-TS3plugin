// Starts the fake ClientQuery server and prints its port (for manual and
// cross-language tests). Usage: node test/run-mock.js [apiKey]
import { startMockTs3 } from './mock-ts3.js';

const mock = await startMockTs3({ apiKey: process.argv[2] || 'KEY-123' });
console.log(mock.port);
process.stdin.on('end', () => mock.close().then(() => process.exit(0)));
process.stdin.resume();
