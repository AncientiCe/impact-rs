import { DetailScreen } from './detail'
import { SettingsScreen } from './settings'
import { Screens } from './types'

export function AppRouter() {
  return (
    <Stack.Navigator>
      <Stack.Screen name={Screens.Detail} component={DetailScreen} />
      <Stack.Screen name="Settings" component={SettingsScreen} />
    </Stack.Navigator>
  )
}
