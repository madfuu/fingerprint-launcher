import '@/index.css';

import { bootstrap } from '@/app/bootstrap';

const container = document.getElementById('root');
if (!container) {
  throw new Error('#root element is missing from index.html');
}

void bootstrap(container);
