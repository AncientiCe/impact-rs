import { PrinterEditor } from './screen'

export function AppNavigator() {
  return (
    <Stack.Navigator>
      <Stack.Screen name="PrinterEditor" component={PrinterEditor} />
    </Stack.Navigator>
  )
}
