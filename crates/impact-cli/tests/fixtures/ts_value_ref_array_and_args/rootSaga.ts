import { mySaga } from './saga'

function otherSaga() {
  return null
}

export function rootSaga() {
  const sagas = [otherSaga, mySaga]
  return sagas.map(fn => spawn(fn))
}
