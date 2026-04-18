/* @refresh reload */
import { render } from 'solid-js/web';
import './styles/global.css';
import { App } from './App';

const root = document.getElementById('root');
if (!root) {
  throw new Error('#root element not found');
}
render(() => <App />, root);
