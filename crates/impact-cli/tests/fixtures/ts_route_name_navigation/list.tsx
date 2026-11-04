import { Screens } from './types'

export function ListScreen({ navigation }) {
  return navigation.navigate(Screens.Detail, { id: 1 })
}
