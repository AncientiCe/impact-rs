const { print: emit } = require('./printer')

function legacy() {
  return emit('legacy')
}

module.exports = { legacy }
