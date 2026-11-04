import defaultOptions from './options'

export const SpreadStack = () => (
  <Stack.Navigator screenOptions={{ ...defaultOptions, detachPreviousScreen: true }} />
)
