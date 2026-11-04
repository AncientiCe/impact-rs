import { DetailScreen } from './detail'
import { Screens } from './types'

function renderDetail() {
  return <Stack.Screen name={Screens.Detail} component={DetailScreen} />
}
