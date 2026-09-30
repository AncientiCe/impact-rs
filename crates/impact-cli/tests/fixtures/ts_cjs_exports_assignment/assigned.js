const { print } = require('./printer')

exports.first = function first() {
  return print('a')
}

module.exports.second = function () {
  return print('b')
}

exports.third = () => print('c')
